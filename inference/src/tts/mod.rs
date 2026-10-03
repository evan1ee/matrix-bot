pub mod kokoro;
pub mod opus;

#[allow(unused_imports)]
pub use kokoro::{
    ensure_model_downloaded, model_present, resolve_kokoro_dir, resolve_model_dir, synthesize,
    Speech, TtsError, KOKORO_REQUIRED_FILES,
};
pub use opus::encode_ogg_opus;
