//! Microphone capture. The stream opens on key-down and closes on release, so the mic
//! indicator only shows while you are dictating.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::JoinHandle;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use parking_lot::Mutex;

pub const RATE: u32 = 16_000;

pub fn microphones() -> Vec<String> {
    let host = cpal::default_host();
    host.input_devices()
        .map(|ds| ds.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}

pub struct Recording {
    stop: Arc<AtomicBool>,
    samples: Arc<Mutex<Vec<f32>>>,
    rate: u32,
    thread: Option<JoinHandle<()>>,
}

impl Recording {
    /// Start capturing. `level` gets a 0..1 loudness about 30 times a second.
    pub fn start(
        device: Option<&str>,
        level: impl Fn(f32) + Send + 'static,
    ) -> anyhow::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let samples = Arc::new(Mutex::new(Vec::with_capacity(RATE as usize * 60)));
        let (tx, rx) = mpsc::channel::<anyhow::Result<u32>>();
        let device = device.map(str::to_string);
        let (stop2, samples2) = (stop.clone(), samples.clone());

        // cpal streams are not Send on every platform, so the stream lives on its own thread.
        let thread = std::thread::spawn(move || {
            let run = || -> anyhow::Result<(cpal::Stream, u32)> {
                let host = cpal::default_host();
                let dev = match &device {
                    Some(name) => host
                        .input_devices()?
                        .find(|d| d.name().ok().as_deref() == Some(name.as_str()))
                        .or_else(|| host.default_input_device()),
                    None => host.default_input_device(),
                }
                .ok_or_else(|| anyhow::anyhow!("No microphone found"))?;
                let config = dev.default_input_config()?;
                let rate = config.sample_rate().0;
                let channels = config.channels() as usize;
                let fmt = config.sample_format();
                let cfg: cpal::StreamConfig = config.into();
                let window = (rate / 30) as usize;
                let mut acc = 0f32;
                let mut n = 0usize;
                let mut push = move |mono: &mut dyn Iterator<Item = f32>| {
                    let mut buf = samples2.lock();
                    for s in mono {
                        buf.push(s);
                        acc += s * s;
                        n += 1;
                        if n >= window {
                            let rms = (acc / n as f32).sqrt();
                            level((rms * 6.0).min(1.0));
                            acc = 0.0;
                            n = 0;
                        }
                    }
                };
                let err = |e| log::error!("microphone error: {e}");
                let stream = match fmt {
                    cpal::SampleFormat::F32 => dev.build_input_stream(
                        &cfg,
                        move |d: &[f32], _: &_| {
                            push(
                                &mut d
                                    .chunks(channels)
                                    .map(|f| f.iter().sum::<f32>() / channels as f32),
                            )
                        },
                        err,
                        None,
                    )?,
                    cpal::SampleFormat::I16 => dev.build_input_stream(
                        &cfg,
                        move |d: &[i16], _: &_| {
                            push(&mut d.chunks(channels).map(|f| {
                                f.iter().map(|&s| s as f32 / 32768.0).sum::<f32>() / channels as f32
                            }))
                        },
                        err,
                        None,
                    )?,
                    cpal::SampleFormat::I32 => dev.build_input_stream(
                        &cfg,
                        move |d: &[i32], _: &_| {
                            push(&mut d.chunks(channels).map(|f| {
                                f.iter().map(|&s| s as f32 / 2_147_483_648.0).sum::<f32>()
                                    / channels as f32
                            }))
                        },
                        err,
                        None,
                    )?,
                    other => anyhow::bail!("Unsupported microphone format {other:?}"),
                };
                stream.play()?;
                Ok((stream, rate))
            };
            match run() {
                Ok((stream, rate)) => {
                    let _ = tx.send(Ok(rate));
                    while !stop2.load(Ordering::Relaxed) {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    drop(stream);
                }
                Err(e) => {
                    let _ = tx.send(Err(e));
                }
            }
        });
        let rate = rx
            .recv()
            .map_err(|_| anyhow::anyhow!("microphone thread died"))??;
        Ok(Self {
            stop,
            samples,
            rate,
            thread: Some(thread),
        })
    }

    pub fn seconds(&self) -> f32 {
        self.samples.lock().len() as f32 / self.rate as f32
    }

    /// Stop and return 16 kHz mono audio.
    pub fn finish(mut self) -> Vec<f32> {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let raw = std::mem::take(&mut *self.samples.lock());
        resample(&raw, self.rate, RATE)
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Box-filter then linear-interpolate. Plenty for speech going down to 16 kHz.
pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let width = ratio.round().max(1.0) as usize;
    let filtered: Vec<f32> = if width > 1 {
        let mut out = Vec::with_capacity(input.len());
        let mut sum = 0f32;
        for i in 0..input.len() {
            sum += input[i];
            if i >= width {
                sum -= input[i - width];
            }
            out.push(sum / width.min(i + 1) as f32);
        }
        out
    } else {
        input.to_vec()
    };
    let n = (input.len() as f64 / ratio) as usize;
    (0..n)
        .map(|i| {
            let pos = i as f64 * ratio;
            let j = pos.floor() as usize;
            let frac = (pos - j as f64) as f32;
            let a = filtered[j.min(filtered.len() - 1)];
            let b = filtered[(j + 1).min(filtered.len() - 1)];
            a + (b - a) * frac
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn resample_keeps_duration() {
        let one_second = vec![0.1f32; 48_000];
        assert_eq!(super::resample(&one_second, 48_000, 16_000).len(), 16_000);
        assert_eq!(super::resample(&one_second, 16_000, 16_000).len(), 48_000);
    }
}
