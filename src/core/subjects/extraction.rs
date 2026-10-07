use super::{Artifact, Evidence, normalized};
use anyhow::Result;

/// Compact evidence from one DINO forward pass. Raw patch tensors are discarded.
/// CLS remains full precision for existing consumers; subject evidence matches
/// the reference's float16 cache precision on cold and warm runs alike.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct DenseEmbedding {
    pub global: Vec<f32>,
    pub regions: Vec<Evidence>,
}
impl DenseEmbedding {
    pub fn from_tokens(tokens: &[f32], width: usize) -> Result<Self> {
        anyhow::ensure!(
            width > 0
                && width <= 2048
                && tokens.len().is_multiple_of(width)
                && tokens.len() / width >= 2
                && tokens.iter().all(|v| v.is_finite()),
            "Expected finite CLS and patch descriptors of matching width"
        );
        let global = normalized(&tokens[..width]);
        let patches: Vec<_> = tokens[width..]
            .chunks_exact(width)
            .map(normalized)
            .collect();
        let count = patches.len();
        let side = count.isqrt();
        let rectangles = if side * side == count {
            let cells = side.min(4);
            let edges: Vec<_> = (0..=cells).map(|i| i * side / cells).collect();
            let mut rectangles = Vec::with_capacity(cells * cells);
            for row in 0..cells {
                for column in 0..cells {
                    rectangles.push((edges[column], edges[row], edges[column + 1], edges[row + 1]));
                }
            }
            rectangles
        } else {
            vec![(0, 0, count, 1)]
        };
        let (columns, rows) = if side * side == count {
            (side, side)
        } else {
            (count, 1)
        };
        let mut regions = Vec::with_capacity(rectangles.len());
        for (left, top, right, bottom) in rectangles {
            let area = (right - left) * (bottom - top);
            let mut mean = vec![0.0; width];
            for y in top..bottom {
                for x in left..right {
                    for (sum, value) in mean.iter_mut().zip(&patches[y * columns + x]) {
                        *sum += value;
                    }
                }
            }
            for v in &mut mean {
                *v /= area as f32;
            }
            regions.push(Evidence {
                descriptor: quantized(&normalized(&mean)),
                bbox: [
                    left as f32 / columns as f32,
                    top as f32 / rows as f32,
                    right as f32 / columns as f32,
                    bottom as f32 / rows as f32,
                ],
                area: area as f32 / count as f32,
                kind: "region".into(),
            });
        }
        Ok(Self { global, regions })
    }
    /// Call only after the shared face service has supplied all face evidence.
    pub fn into_artifact(self, faces: Vec<Evidence>) -> Artifact {
        Artifact {
            global: quantized(&self.global),
            subjects: self.regions,
            faces,
        }
    }
}
fn quantized(values: &[f32]) -> Vec<f32> {
    values
        .iter()
        .map(|v| half::f16::from_f32(*v).to_f32())
        .collect()
}
