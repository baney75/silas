//! A headless sync node for trying sync between two machines without the app.
//!
//!   cargo run --example sync-node -- --dir /tmp/a --code-file /tmp/a/code --add Parish1 --secs 30
//!   cargo run --example sync-node -- --dir /tmp/b --code-file /tmp/b/code --peer host-a:47823 --until Parish1
//!
//! `--code-file` holds the pairing code. If the file is missing, the node makes a key and writes the
//! code there (mode 0600); it never prints it. Copy that file to the other node to pair it.
//! Flags: --dir D, --port P (default 47823), --name N, --peer ADDR (repeat), --add WORD,
//! --remove WORD, --until WORD (exit 0 once WORD is here), --until-gone WORD, --secs N (run time or
//! timeout, default 60), --discover (also use mDNS).

use std::io::Write;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use polycarp_voice::sync_engine::{FileDict, Options, SyncEngine};
use polycarp_voice::sync_key::FileKeyStore;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("polycarp_voice=info,sync_node=info"),
    )
    .init();
    let mut args = std::env::args().skip(1);
    let (mut dir, mut port, mut name, mut code_file) = (None, 47823u16, None, None);
    let (mut peers, mut add, mut remove) = (vec![], vec![], vec![]);
    let (mut until, mut until_gone, mut secs, mut discover) = (None, None, 60u64, false);
    while let Some(a) = args.next() {
        let mut val = || {
            args.next()
                .ok_or_else(|| anyhow::anyhow!("{a} needs a value"))
        };
        match a.as_str() {
            "--dir" => dir = Some(PathBuf::from(val()?)),
            "--port" => port = val()?.parse()?,
            "--name" => name = Some(val()?),
            "--code-file" => code_file = Some(PathBuf::from(val()?)),
            "--peer" => peers.push(val()?),
            "--add" => add.push(val()?),
            "--remove" => remove.push(val()?),
            "--until" => until = Some(val()?),
            "--until-gone" => until_gone = Some(val()?),
            "--secs" => secs = val()?.parse()?,
            "--discover" => discover = true,
            _ => anyhow::bail!("unknown flag {a}"),
        }
    }
    let dir = dir.ok_or_else(|| anyhow::anyhow!("--dir is required"))?;
    let code_file = code_file.ok_or_else(|| anyhow::anyhow!("--code-file is required"))?;
    std::fs::create_dir_all(&dir)?;

    let dict = Arc::new(FileDict::open(dir.join("dictionary.json")));
    let mut o = Options::new(
        dir.clone(),
        Arc::new(FileKeyStore::new(dir.join("sync-key"))),
        dict.clone(),
    );
    o.port = Some(port);
    o.bind = IpAddr::V4(Ipv4Addr::UNSPECIFIED);
    o.discovery = discover;
    if let Some(n) = name {
        o.default_name = n;
    }
    let engine = SyncEngine::new(o);

    if code_file.exists() {
        engine.join(std::fs::read_to_string(&code_file)?.trim())?;
        log::info!("sync-node: joined with the code in {}", code_file.display());
    } else {
        let code = engine.pairing_code()?;
        let mut f = std::fs::OpenOptions::new();
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut f, 0o600);
        f.write(true)
            .create_new(true)
            .open(&code_file)?
            .write_all(code.code.as_bytes())?;
        log::info!(
            "sync-node: made a key; the code is in {}",
            code_file.display()
        );
    }
    if !add.is_empty() || !remove.is_empty() {
        dict.edit(|d| {
            for w in &add {
                d.add_term(w, "user");
            }
            for w in &remove {
                d.remove_term(w);
            }
        });
        log::info!("sync-node: edited here: added {add:?}, removed {remove:?}");
        engine.local_changed();
    }

    // Peers last: adding one syncs at once, so the edit above goes in that first exchange.
    for p in &peers {
        engine.add_peer(p)?;
    }
    let s = engine.status();
    log::info!(
        "sync-node: {} ({}) listening on port {:?}, peers {:?}",
        s.name,
        &s.device_id[..8],
        s.port,
        s.peers
    );

    let has = |w: &str| dict.get().terms.iter().any(|t| t.text == w);
    let until_at = Instant::now() + Duration::from_secs(secs);
    let mut ok = until.is_none() && until_gone.is_none();
    while Instant::now() < until_at {
        if until.as_deref().is_some_and(has) || until_gone.as_deref().is_some_and(|w| !has(w)) {
            ok = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let terms: Vec<String> = dict.get().terms.iter().map(|t| t.text.clone()).collect();
    log::info!("sync-node: terms now {terms:?}");
    engine.stop();
    if !ok {
        anyhow::bail!("timed out after {secs} s");
    }
    Ok(())
}
