//! Throwaway spike: can this repo speak, using only what it already depends on?
//!
//! Deliberately an example rather than a third binary, so it cannot disturb
//! `default-run`. Delete it once the question is answered.
//!
//! The question is narrow. sherpa-onnx is already a dependency for speech to
//! text, and the prebuilt C API it ships already contains `OfflineTtsKokoroImpl`
//! and `KokoroMultiLangLexicon`. If that works, text to speech needs no new
//! dependency, no second ML runtime, and no hand-written grapheme-to-phoneme
//! step, which the raw ONNX model would have forced: it takes phoneme ids, and
//! its own model card points at a Python library for producing them.
//!
//! Drop any sherpa-onnx TTS model under `data/tts_models/` and run:
//!
//!   cargo run -p matrix-bot --example tts_spike
//!   cargo run -p matrix-bot --example tts_spike -- 0,10,44
//!
//! Every model found is run over the same lines, so the output files sit side
//! by side and can be compared by ear.

use std::path::{Path, PathBuf};

use sherpa_onnx::{
    GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsKokoroModelConfig,
    OfflineTtsMatchaModelConfig, OfflineTtsModelConfig, OfflineTtsVitsModelConfig,
};

/// No streaming callback; only the finished audio is wanted.
type NoCallback = fn(&[f32], f32) -> bool;

const MODELS_ROOT: &str = "data/tts_models";

/// Lines shaped like what this bot actually says, minus the emoji, which a
/// phonemiser has no sensible reading for.
const SAMPLES: &[(&str, &str)] = &[
    (
        "en",
        "Good morning. You have three tasks due today, and one recurring bill is overdue.",
    ),
    (
        "zh",
        "早安。你今天有三件待办事项，其中两件已经逾期。本月支出比上个月多了百分之十二。",
    ),
    // The case a per-language lexicon does not solve: one call, one voice, two
    // languages. Listen to this one first. It decides whether routing per
    // message is enough, or whether the text has to be segmented.
    ("mixed", "这个 PR 已经 merge 了，还有两个 review 在等你。"),
    // Real bot output is full of digits, dates and percentages. Kokoro's
    // Chinese build ships FSTs for exactly this, wired up as `rule_fsts`
    // below; without them a number is read digit by digit, or in English.
    (
        "digits",
        "2026年9月20日，你有3件待办事项，本月支出增加了12.5%，共计1580元。",
    ),
];

/// Text-normalisation rules, in the order sherpa-onnx documents for this model.
///
/// These turn "12.5%" and "2026年9月20日" into something a Mandarin voice can
/// actually say. They ship inside the model directory and are easy to miss,
/// because nothing fails without them: the audio is simply wrong.
const RULE_FSTS: &[&str] = &["date-zh.fst", "number-zh.fst", "phone-zh.fst"];

/// Which family a model directory belongs to, decided by what is in it rather
/// than by its name, so a renamed folder still works.
enum Family {
    Kokoro,
    Vits,
    Matcha { acoustic: PathBuf, vocoder: PathBuf },
}

/// The path, but only if that file is actually present.
///
/// Every one of these fields is optional in sherpa-onnx, and handing it a path
/// to something missing fails less helpfully than leaving it unset.
fn opt(dir: &Path, name: &str) -> Option<String> {
    let path = dir.join(name);
    path.exists().then(|| path.to_string_lossy().into_owned())
}

fn first_match(dir: &Path, prefix: &str) -> Option<PathBuf> {
    let mut hits: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(prefix) && name.ends_with(".onnx"))
        })
        .collect();
    hits.sort();
    hits.into_iter().next()
}

fn detect(dir: &Path) -> Option<Family> {
    // Kokoro is the one carrying a bank of style vectors.
    if dir.join("voices.bin").exists() && dir.join("model.onnx").exists() {
        return Some(Family::Kokoro);
    }

    // Matcha splits the acoustic model from the vocoder, and sherpa-onnx ships
    // the vocoder as a separate download, so it may simply not be here.
    if let Some(acoustic) = first_match(dir, "model-steps-") {
        let vocoder = first_match(dir, "vocos")
            .or_else(|| first_match(dir, "hifigan"))
            .or_else(|| first_match(Path::new(MODELS_ROOT), "vocos"))
            .or_else(|| first_match(Path::new(MODELS_ROOT), "hifigan"));
        return vocoder.map(|vocoder| Family::Matcha { acoustic, vocoder });
    }

    if dir.join("model.onnx").exists() {
        return Some(Family::Vits);
    }

    None
}

fn build_config(dir: &Path, family: &Family) -> OfflineTtsConfig {
    // Kokoro ships one lexicon per language, and sherpa-onnx takes them as a
    // comma-separated list and picks per utterance. That is the whole of
    // "per-language routing" for this engine: a path, not an abstraction.
    // One English lexicon only. Passing both the US and GB files makes
    // sherpa-onnx log a "Duplicated word" line for every entry they share,
    // which is most of them, and the second one is ignored anyway.
    let kokoro_lexicons = ["lexicon-us-en.txt", "lexicon-zh.txt"]
        .iter()
        .filter_map(|name| opt(dir, name))
        .collect::<Vec<String>>()
        .join(",");

    let model = match family {
        Family::Kokoro => OfflineTtsModelConfig {
            kokoro: OfflineTtsKokoroModelConfig {
                model: opt(dir, "model.onnx"),
                voices: opt(dir, "voices.bin"),
                tokens: opt(dir, "tokens.txt"),
                // espeak-ng data: the grapheme-to-phoneme step, already solved.
                data_dir: opt(dir, "espeak-ng-data"),
                lexicon: (!kokoro_lexicons.is_empty()).then_some(kokoro_lexicons),
                dict_dir: opt(dir, "dict"),
                ..Default::default()
            },
            ..Default::default()
        },
        Family::Vits => OfflineTtsModelConfig {
            vits: OfflineTtsVitsModelConfig {
                model: opt(dir, "model.onnx"),
                tokens: opt(dir, "tokens.txt"),
                lexicon: opt(dir, "lexicon.txt"),
                data_dir: opt(dir, "espeak-ng-data"),
                dict_dir: opt(dir, "dict"),
                ..Default::default()
            },
            ..Default::default()
        },
        Family::Matcha { acoustic, vocoder } => OfflineTtsModelConfig {
            matcha: OfflineTtsMatchaModelConfig {
                acoustic_model: Some(acoustic.to_string_lossy().into_owned()),
                vocoder: Some(vocoder.to_string_lossy().into_owned()),
                tokens: opt(dir, "tokens.txt"),
                lexicon: opt(dir, "lexicon.txt"),
                data_dir: opt(dir, "espeak-ng-data"),
                dict_dir: opt(dir, "dict"),
                ..Default::default()
            },
            ..Default::default()
        },
    };

    let rule_fsts = RULE_FSTS
        .iter()
        .filter_map(|name| opt(dir, name))
        .collect::<Vec<String>>()
        .join(",");

    OfflineTtsConfig {
        model: OfflineTtsModelConfig {
            num_threads: 2,
            ..model
        },
        rule_fsts: (!rule_fsts.is_empty()).then_some(rule_fsts),
        ..Default::default()
    }
}

fn run(dir: &Path, sids: &[i32]) {
    let name = dir
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();

    let Some(family) = detect(dir) else {
        println!("{name}: no recognised model layout, skipping");
        return;
    };

    let Some(tts) = OfflineTts::create(&build_config(dir, &family)) else {
        println!("{name}: OfflineTts::create failed, check the files in that folder");
        return;
    };

    println!(
        "\n{name}\n  speakers {}, {} Hz",
        tts.num_speakers(),
        tts.sample_rate()
    );

    for &sid in sids {
        for (tag, text) in SAMPLES {
            let started = std::time::Instant::now();
            let generation = GenerationConfig {
                sid,
                ..Default::default()
            };

            let Some(audio) = tts.generate_with_config(text, &generation, None::<NoCallback>)
            else {
                println!("  {tag} sid{sid}: generation failed");
                continue;
            };

            let out = format!("{MODELS_ROOT}/out-{name}-{tag}-sid{sid}.wav");
            let seconds = audio.samples().len() as f32 / audio.sample_rate().max(1) as f32;
            let took = started.elapsed().as_secs_f32();

            if !audio.save(&out) {
                println!("  {tag} sid{sid}: could not write {out}");
                continue;
            }

            // Also write what actually goes to Matrix. The WAV above is for
            // auditioning in any player; this is the shipping format, and
            // writing both here means the encoder is exercised against real
            // speech rather than only the synthetic samples in its unit tests.
            let ogg_note = match matrix_bot::modules::tts::opus::encode_ogg_opus(
                audio.samples(),
                audio.sample_rate(),
            ) {
                Ok(bytes) => {
                    let ogg_path = out.replace(".wav", ".ogg");
                    let wav_len = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
                    match std::fs::write(&ogg_path, &bytes) {
                        Ok(()) => format!(
                            "  ogg {:>6} B vs wav {:>7} B ({:.0}x smaller)",
                            bytes.len(),
                            wav_len,
                            wav_len as f32 / bytes.len().max(1) as f32
                        ),
                        Err(err) => format!("  ogg write failed: {err}"),
                    }
                }
                Err(err) => format!("  ogg encode failed: {err}"),
            };

            let realtime = seconds / took.max(f32::EPSILON);
            println!(
                "  {tag:<6} sid{sid:<4} {seconds:>5.1}s audio in {took:>5.1}s ({realtime:.1}x realtime){ogg_note}"
            );
        }
    }
}

fn main() {
    let sids: Vec<i32> = std::env::args()
        .nth(1)
        .map(|arg| {
            arg.split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect()
        })
        .unwrap_or_else(|| vec![0]);

    let root = Path::new(MODELS_ROOT);
    if !root.exists() {
        eprintln!("no {MODELS_ROOT} directory, extract a sherpa-onnx TTS model there first");
        std::process::exit(1);
    }

    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root)
        .expect("models root must be readable")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();

    if dirs.is_empty() {
        eprintln!("{MODELS_ROOT} has no model directories yet");
        std::process::exit(1);
    }

    println!("speakers to try: {sids:?}");
    for dir in &dirs {
        run(dir, &sids);
    }
    println!("\nwav files are in {MODELS_ROOT}/");
}
