# Bench set v1: licenses and attributions

Every source below allows redistribution, so the whole set ships in the
`bench-set-v1` release. Each manifest row carries the license that applies to that
clip in its `license` field.

| Clips | Source | License | Redistributed |
| --- | --- | --- | --- |
| clean (60), longform-0001 to 0005 | LibriSpeech test-clean and test-other | CC BY 4.0 | yes |
| clean, longform `ref_formatted` | LibriSpeech-PC punctuated transcripts, hand edited | CC BY 4.0 | yes |
| noise (60) | LibriSpeech test-clean + DEMAND or FSD50K noise | CC BY 4.0 | yes |
| accent (50) | EdAcc test split | CC BY-SA 4.0 | yes, under CC BY-SA 4.0 |
| dictation, code, command, longform-0006 to 0010 | Text written for this set, voiced with Kokoro-82M | CC BY 4.0 | yes |

## LibriSpeech

V. Panayotov, G. Chen, D. Povey and S. Khudanpur, "LibriSpeech: an ASR corpus based on
public domain audio books," ICASSP 2015. <https://www.openslr.org/12>. CC BY 4.0. The
audio comes from LibriVox recordings of public domain books. Clips are copied unchanged
(FLAC to 16-bit WAV, same samples); longform passages join consecutive utterances of one
chapter with 0.35 s of silence.

## LibriSpeech-PC

A. Meister, M. Novikov, N. Karpov, E. Bakhturina, V. Lavrukhin and B. Ginsburg,
"LibriSpeech-PC: Benchmark for Evaluation of Punctuation and Capitalization Capabilities
of end-to-end ASR Models," arXiv:2310.02943, 2023. <https://www.openslr.org/145>.
CC BY 4.0. Its punctuated text is the starting point for `ref_formatted` on LibriSpeech
clips; quotes were dropped and other edits made by hand (`bench/sources/refs.json`).

## EdAcc

R. Sanabria, N. Bogoychev, N. Markl, A. Carmantini, O. Klejch and P. Bell, "The Edinburgh
International Accents of English Corpus: Towards the Democratization of English ASR,"
ICASSP 2023. <https://groups.inf.ed.ac.uk/edacc/>, Hugging Face `edinburghcstr/edacc`
(not gated). CC BY-SA 4.0. The accent clips are resampled from 32 kHz to 16 kHz and are
redistributed under CC BY-SA 4.0; anything you make from them must carry the same license.
`ref_formatted` for these clips is our punctuated cleanup of the EdAcc transcript and is
shared under CC BY-SA 4.0 as well.

## DEMAND

J. Thiemann, N. Ito and E. Vincent, "The Diverse Environments Multi-channel Acoustic Noise
Database (DEMAND)," Zenodo record 1227121, <https://zenodo.org/records/1227121>. The
Zenodo record lists CC BY 4.0. Older copies of the project page listed CC BY-SA 3.0; that
license also allows redistribution, and if it applied, the 32 mixes that use DEMAND
(`cafe` and `street`) would carry CC BY-SA 3.0. Used: PCAFETER (cafeteria) and STRAFFIC
(street traffic), 16 kHz, channels 1 and 9.

## FSD50K clips (CC0)

E. Fonseca, X. Favory, J. Pons, F. Font and X. Serra, "FSD50K: An Open Dataset of
Human-Labeled Sound Events," IEEE/ACM TASLP 2022. Clips come from Freesound and keep
their own licenses; only CC0 clips are used, fetched from the Hugging Face mirror
`Fhrozen/FSD50k`:

| Noise | FSD50K clip | Freesound title | Uploader | License |
| --- | --- | --- | --- | --- |
| keyboard | dev 155910 | LaptopKeyboard.wav | tiptoe84 | CC0 1.0 |
| keyboard | dev 378389 | Keyboard.wav | 13FPanska_Stranska_Michaela | CC0 1.0 |
| fan | eval 118767 | noisy ventilation fan.WAV | thedapperdan | CC0 1.0 |
| fan | eval 395734 | Extractor Fan.wav | PapercutterJohn | CC0 1.0 |

## Synthesized speech

Text in `bench/sources/synth.toml` was written for this set and is licensed CC BY 4.0
(attribution: the Silas project). It is voiced with Kokoro-82M by hexgrad
(<https://huggingface.co/hexgrad/Kokoro-82M>, Apache 2.0) through kokoro-onnx by
thewh1teagle (MIT). Neither license restricts the generated audio, which is also
released under CC BY 4.0.
