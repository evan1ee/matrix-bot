use std::path::Path;

/// Checks whether the embedding model file is installed in the target directory.
pub fn models_present(dir: &Path) -> bool {
    dir.join("model.onnx").is_file() || dir.join("bge-small-en-v1.5.onnx").is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_models_present_detects_files() {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let tmp = std::env::temp_dir().join(format!("test_embed_models_{}", now));
        std::fs::create_dir_all(&tmp).unwrap();

        assert!(!models_present(&tmp));

        std::fs::write(tmp.join("model.onnx"), b"fake onnx").unwrap();
        assert!(models_present(&tmp));

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
