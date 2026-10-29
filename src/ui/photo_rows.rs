//! Justified rows for the photo grids: every photo in a row shares one
//! height and takes the width its shape needs, and each full row is scaled
//! to fill the grid's width. A portrait photo gets a narrow tile and a
//! landscape one a wide tile, so no tile carries empty bars beside or above
//! its photo, and a row holds as many photos as their shapes allow.
//!
//! The layout depends only on the tiles' shapes, the width and the target
//! height, so it is stable from frame to frame and rows can be virtualized.
use std::ops::Range;

/// The widest and the narrowest a tile's photo gets, over its height.
/// Beyond them, a panorama or a very tall crop is fitted inside its slot.
const ASPECTS: std::ops::RangeInclusive<f32> = 0.5..=2.5;
/// The most a full row stretches its photos past the target height to fill
/// the width.
const STRETCH: f32 = 1.35;
/// A section's last row fills the width only when that barely stretches it;
/// otherwise it keeps the target height and leaves the rest of the row.
const LAST_STRETCH: f32 = 1.1;

/// One tile to place: the photos it shows side by side and the fixed width
/// around them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Tile {
    /// The photos' widths over their heights, added up.
    pub aspect: f32,
    /// Width that does not grow with the photos: padding and gaps.
    pub extra: f32,
    /// The narrowest the tile gets, so its header and pills still fit;
    /// narrower photos are centred in it.
    pub min_width: f32,
}

impl Tile {
    /// Its width when its photos are `height` tall.
    pub fn width(&self, height: f32) -> f32 {
        (self.extra + self.aspect * height).max(self.min_width)
    }
}

/// One row of tiles.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct PhotoRow {
    /// The tiles it holds, by index.
    pub range: Range<usize>,
    /// The height of its photos.
    pub height: f32,
    /// Each tile's left edge and width, from the row's left edge.
    pub tiles: Vec<(f32, f32)>,
}

/// A photo's width over its height as shown, with its EXIF orientation and
/// `turns` quarter turns applied. Unknown sizes count as 4:3.
pub(super) fn aspect(width: u32, height: u32, orientation: u8, turns: u8) -> f32 {
    if width == 0 || height == 0 {
        return 4.0 / 3.0;
    }
    let sideways = (5..=8).contains(&orientation) != (turns % 2 == 1);
    let (w, h) = if sideways {
        (height, width)
    } else {
        (width, height)
    };
    (w as f32 / h as f32).clamp(*ASPECTS.start(), *ASPECTS.end())
}

/// The photo height at which `tiles` side by side, `gap` apart, fill
/// `width` exactly. When every tile is at its narrowest no height does:
/// infinite if they never reach the width, zero if they never fit in it.
fn fill_height(tiles: &[Tile], width: f32, gap: f32) -> f32 {
    let room = width - gap * tiles.len().saturating_sub(1) as f32;
    // Solve as if no tile were at its narrowest, then hold the ones that
    // are at that height to their narrowest and solve again. Each pass
    // only lowers the height, so a tile held once stays held.
    let mut held = vec![false; tiles.len()];
    loop {
        let (fixed, aspect) =
            tiles
                .iter()
                .zip(&held)
                .fold((0.0, 0.0), |(fixed, aspect), (tile, held)| {
                    if *held {
                        (fixed + tile.min_width, aspect)
                    } else {
                        (fixed + tile.extra, aspect + tile.aspect)
                    }
                });
        if aspect <= 0.0 {
            return if fixed <= room { f32::INFINITY } else { 0.0 };
        }
        let height = ((room - fixed) / aspect).max(0.0);
        let mut changed = false;
        for (tile, held) in tiles.iter().zip(&mut held) {
            if !*held && tile.extra + tile.aspect * height < tile.min_width {
                *held = true;
                changed = true;
            }
        }
        if !changed {
            return height;
        }
    }
}

/// Lays `tiles` out in rows `width` wide, `gap` apart, with photos about
/// `target` tall. Each row takes tiles while they fit at the target height,
/// then takes one more if that brings its height closer to the target, and
/// is scaled to fill the width. A tile wider than the grid on its own is
/// shrunk to fit.
pub(super) fn justify(tiles: &[Tile], width: f32, target: f32, gap: f32) -> Vec<PhotoRow> {
    let width = width.max(1.0);
    let target = target.max(1.0);
    // A tile never asks to be wider than the grid.
    let tiles: Vec<Tile> = tiles
        .iter()
        .map(|t| Tile {
            min_width: t.min_width.min(width),
            ..*t
        })
        .collect();
    let at_target = |range: Range<usize>| {
        tiles[range.clone()]
            .iter()
            .map(|t| t.width(target))
            .sum::<f32>()
            + gap * (range.len() - 1) as f32
    };
    let mut rows = Vec::new();
    let mut start = 0;
    while start < tiles.len() {
        let mut end = start + 1;
        while end < tiles.len() && at_target(start..end + 1) <= width {
            end += 1;
        }
        let fits = fill_height(&tiles[start..end], width, gap);
        let height = if end == tiles.len() {
            // The last row fills the width only when it is nearly full.
            if fits <= target * LAST_STRETCH {
                fits
            } else {
                target
            }
        } else {
            // One more tile shrinks the row below the target; take it when
            // that lands nearer the target than stretching the row does.
            let more = fill_height(&tiles[start..end + 1], width, gap);
            if fits >= target
                && more.is_finite()
                && more * STRETCH >= target
                && target / more < fits / target
            {
                end += 1;
                more
            } else {
                fits.min(target * STRETCH)
            }
        };
        let mut left = 0.0;
        let placed = tiles[start..end]
            .iter()
            .map(|tile| {
                let w = tile.width(height).min(width);
                let at = (left, w);
                left += w + gap;
                at
            })
            .collect();
        rows.push(PhotoRow {
            range: start..end,
            height,
            tiles: placed,
        });
        start = end;
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    const LANDSCAPE: f32 = 1.5;
    const PORTRAIT: f32 = 2.0 / 3.0;

    fn photos(aspects: &[f32]) -> Vec<Tile> {
        aspects
            .iter()
            .map(|a| Tile {
                aspect: *a,
                extra: 8.0,
                min_width: 60.0,
            })
            .collect()
    }

    fn per_row(rows: &[PhotoRow]) -> Vec<usize> {
        rows.iter().map(|r| r.range.len()).collect()
    }

    /// Every full row ends at the right edge; every row is in order, covers
    /// its tiles once, and keeps its tiles apart.
    fn check(rows: &[PhotoRow], count: usize, width: f32, gap: f32) {
        let mut next = 0;
        for (i, row) in rows.iter().enumerate() {
            assert_eq!(row.range.start, next);
            assert_eq!(row.tiles.len(), row.range.len());
            next = row.range.end;
            for pair in row.tiles.windows(2) {
                assert!((pair[1].0 - (pair[0].0 + pair[0].1 + gap)).abs() < 0.01);
            }
            let (left, w) = *row.tiles.last().unwrap();
            assert!(left + w <= width + 0.01, "row {i} overflows");
            if i + 1 < rows.len() && row.height < 1000.0 {
                assert!(
                    (left + w - width).abs() < 0.5 || row.range.len() == 1,
                    "row {i} ends at {} of {width}",
                    left + w
                );
            }
        }
        assert_eq!(next, count);
    }

    #[test]
    fn aspect_follows_orientation_and_turns() {
        assert_eq!(aspect(6000, 4000, 1, 0), 1.5);
        assert_eq!(aspect(6000, 4000, 6, 0), 4000.0 / 6000.0);
        assert_eq!(aspect(6000, 4000, 1, 1), 4000.0 / 6000.0);
        assert_eq!(aspect(6000, 4000, 6, 3), 1.5);
        assert_eq!(aspect(6000, 4000, 1, 2), 1.5);
        assert_eq!(aspect(0, 4000, 1, 0), 4.0 / 3.0);
        // A panorama and a sliver are fitted inside a bounded slot.
        assert_eq!(aspect(12000, 2000, 1, 0), 2.5);
        assert_eq!(aspect(500, 4000, 1, 0), 0.5);
    }

    #[test]
    fn photos_in_a_row_share_a_height_and_fill_the_width() {
        let tiles = photos(&[
            LANDSCAPE, PORTRAIT, LANDSCAPE, LANDSCAPE, PORTRAIT, LANDSCAPE,
        ]);
        let rows = justify(&tiles, 1000.0, 200.0, 8.0);
        check(&rows, tiles.len(), 1000.0, 8.0);
        for row in &rows {
            for (index, (_, w)) in row.range.clone().zip(&row.tiles) {
                // Each photo exactly fills its slot: width follows its shape.
                assert!((w - tiles[index].extra - tiles[index].aspect * row.height).abs() < 0.01);
            }
        }
        assert!(rows[0].height >= 200.0 / STRETCH && rows[0].height <= 200.0 * STRETCH);
    }

    #[test]
    fn portrait_photos_pack_more_to_a_row_than_landscape_ones() {
        let landscape = justify(&photos(&[LANDSCAPE; 30]), 1200.0, 200.0, 8.0);
        let portrait = justify(&photos(&[PORTRAIT; 30]), 1200.0, 200.0, 8.0);
        assert_eq!(per_row(&landscape)[0], 4);
        assert_eq!(per_row(&portrait)[0], 8);
        check(&landscape, 30, 1200.0, 8.0);
        check(&portrait, 30, 1200.0, 8.0);
    }

    #[test]
    fn the_size_sets_how_many_fit_at_every_window_width() {
        // (window width, photo height) -> landscape photos in a full row.
        let cases = [
            (500.0, 90.0, 3),
            (500.0, 160.0, 2),
            (500.0, 420.0, 1),
            (1200.0, 90.0, 8),
            (1200.0, 160.0, 5),
            (1200.0, 420.0, 2),
            (2400.0, 90.0, 16),
            (2400.0, 160.0, 9),
            (2400.0, 420.0, 4),
        ];
        for (width, height, expected) in cases {
            let rows = justify(&photos(&[LANDSCAPE; 40]), width, height, 8.0);
            check(&rows, 40, width, 8.0);
            assert_eq!(
                per_row(&rows)[0],
                expected,
                "{width} wide at {height}: {:?}",
                per_row(&rows)
            );
        }
        // Larger photos never mean more to a row.
        for width in [500.0, 1200.0, 2400.0] {
            let counts: Vec<usize> = [90.0, 160.0, 250.0, 420.0]
                .iter()
                .map(|h| per_row(&justify(&photos(&[LANDSCAPE; 40]), width, *h, 8.0))[0])
                .collect();
            assert!(counts.windows(2).all(|p| p[0] >= p[1]), "{counts:?}");
        }
    }

    #[test]
    fn a_mix_of_shapes_keeps_rows_near_the_target_height() {
        let mix: Vec<f32> = (0..50)
            .map(|i| match i % 5 {
                0 | 3 => PORTRAIT,
                1 => 1.0,
                _ => LANDSCAPE,
            })
            .collect();
        for width in [480.0, 900.0, 1600.0] {
            for target in [90.0, 160.0, 420.0] {
                let rows = justify(&photos(&mix), width, target, 8.0);
                check(&rows, mix.len(), width, 8.0);
                for row in &rows[..rows.len() - 1] {
                    // A lone wide tile in a narrow window shrinks to fit.
                    if row.range.len() > 1 {
                        assert!(
                            row.height >= target / STRETCH - 0.01
                                && row.height <= target * STRETCH + 0.01,
                            "{width}/{target}: {}",
                            row.height
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_last_row_keeps_the_target_height_when_it_is_short() {
        let rows = justify(&photos(&[LANDSCAPE; 3]), 1200.0, 160.0, 8.0);
        assert_eq!(per_row(&rows), vec![3]);
        assert_eq!(rows[0].height, 160.0);
        let (left, w) = *rows[0].tiles.last().unwrap();
        assert!(left + w < 1200.0);
    }

    #[test]
    fn a_tile_wider_than_the_grid_shrinks_to_fit() {
        let tiles = photos(&[2.5, 2.5, 2.5]);
        let tile = Tile {
            aspect: 7.5,
            extra: 40.0,
            min_width: 160.0,
        };
        let rows = justify(&[tile], 600.0, 160.0, 8.0);
        assert_eq!(rows.len(), 1);
        assert!((rows[0].tiles[0].1 - 600.0).abs() < 0.01);
        assert!(rows[0].height < 160.0);
        check(&justify(&tiles, 300.0, 420.0, 8.0), 3, 300.0, 8.0);
    }

    #[test]
    fn narrow_tiles_keep_their_minimum_width() {
        // A lone portrait photo is narrower than its card's header needs.
        let card = Tile {
            aspect: PORTRAIT,
            extra: 16.0,
            min_width: 170.0,
        };
        let pair = Tile {
            aspect: 2.0 * LANDSCAPE,
            extra: 24.0,
            min_width: 170.0,
        };
        let tiles = [card, pair, card, pair, card, card, pair];
        let rows = justify(&tiles, 1000.0, 120.0, 8.0);
        check(&rows, tiles.len(), 1000.0, 8.0);
        for row in &rows {
            assert!(row.tiles.iter().all(|(_, w)| *w >= 170.0 - 0.01));
        }
        // Tiles at their narrowest everywhere never fill a row: they keep
        // the target height.
        let rows = justify(
            &[Tile {
                aspect: 0.0,
                extra: 0.0,
                min_width: 100.0,
            }; 3],
            1000.0,
            120.0,
            8.0,
        );
        assert_eq!(rows[0].height, 120.0);
    }

    #[test]
    fn a_narrow_grid_never_takes_more_tiles_than_fit_at_their_narrowest() {
        // Organize's narrow board: portrait tiles at their narrowest are
        // still too wide for three to a row.
        let tile = |aspect| Tile {
            aspect,
            extra: 12.0,
            min_width: 104.0,
        };
        let tiles: Vec<Tile> = [PORTRAIT, PORTRAIT, PORTRAIT, LANDSCAPE, PORTRAIT, 1.0]
            .into_iter()
            .map(tile)
            .collect();
        for width in [200.0, 290.0, 381.0] {
            let rows = justify(&tiles, width, 160.0, 6.0);
            check(&rows, tiles.len(), width, 6.0);
            for row in &rows {
                assert!(row.height.is_finite() && row.height > 0.0, "{row:?}");
            }
        }
    }

    #[test]
    fn the_layout_is_the_same_every_time() {
        let tiles = photos(&[LANDSCAPE, PORTRAIT, 1.0, LANDSCAPE, 2.4, PORTRAIT, PORTRAIT]);
        assert_eq!(
            justify(&tiles, 913.0, 177.0, 8.0),
            justify(&tiles, 913.0, 177.0, 8.0)
        );
    }
}
