//! Turns a transformer's per-token hidden states into one sentence vector.

/// Mean-pools one sequence's hidden states over its attention mask, then
/// L2-normalises, which is the pooling `intfloat/multilingual-e5-*` was
/// trained with. `hidden` is `mask.len() * dim` values, token-major.
///
/// Returns `None` when no token is attended to or the result has no usable
/// length (all zeros or non-finite): a vector like that cannot be compared by
/// cosine distance, so the caller must fail the request, not store it.
pub fn mean_pool_normalise(hidden: &[f32], mask: &[i64], dim: usize) -> Option<Vec<f32>> {
    if dim == 0 || hidden.len() != mask.len() * dim {
        return None;
    }

    let mut sum = vec![0.0f32; dim];
    let mut attended = 0usize;
    for (token, &m) in hidden.chunks_exact(dim).zip(mask) {
        if m == 0 {
            continue;
        }
        attended += 1;
        for (acc, v) in sum.iter_mut().zip(token) {
            *acc += v;
        }
    }
    if attended == 0 {
        return None;
    }

    let mean: Vec<f32> = sum.into_iter().map(|v| v / attended as f32).collect();
    let norm = mean.iter().map(|v| v * v).sum::<f32>().sqrt();
    if !norm.is_finite() || norm == 0.0 {
        return None;
    }
    Some(mean.into_iter().map(|v| v / norm).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padding_tokens_do_not_contribute_to_the_mean() {
        // Two real tokens and one padding token carrying a huge value.
        let hidden = [1.0, 0.0, 3.0, 0.0, 100.0, 100.0];
        let pooled = mean_pool_normalise(&hidden, &[1, 1, 0], 2).unwrap();

        assert_eq!(pooled, vec![1.0, 0.0]);
    }

    #[test]
    fn the_result_has_unit_length() {
        let hidden = [3.0, 4.0, 3.0, 4.0];
        let pooled = mean_pool_normalise(&hidden, &[1, 1], 2).unwrap();

        let norm = pooled.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-6, "norm was {norm}");
        assert!((pooled[0] - 0.6).abs() < 1e-6 && (pooled[1] - 0.8).abs() < 1e-6);
    }

    #[test]
    fn a_fully_masked_sequence_has_no_embedding() {
        assert_eq!(mean_pool_normalise(&[1.0, 2.0], &[0], 2), None);
    }

    #[test]
    fn a_zero_mean_has_no_embedding() {
        // Opposite tokens cancel to a zero vector, which cannot be normalised.
        assert_eq!(mean_pool_normalise(&[1.0, -1.0], &[1, 1], 1), None);
    }

    #[test]
    fn a_hidden_state_of_the_wrong_length_is_refused() {
        assert_eq!(mean_pool_normalise(&[1.0, 2.0, 3.0], &[1, 1], 2), None);
    }
}
