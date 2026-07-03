//! Benchmarks for Grid→TextRun conversion and color mapping.
//!
//! Pure CPU benchmarks — no GPU device required.
//! Run with: `cargo bench -p ggterm-render-wgpu`

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use ggterm_core::{Cell, CellFlags, Color, Grid};
use ggterm_render::CursorState;
use ggterm_render::theme::RenderTheme;
use ggterm_render_wgpu::colors::{indexed_to_rgb, map_bg, map_fg};
use ggterm_render_wgpu::converter::{row_to_runs, row_to_text};

// ──── Grid builders ────

fn empty_grid(w: usize, h: usize) -> Grid {
    Grid::new(w, h)
}

fn ascii_grid(w: usize, h: usize) -> Grid {
    let mut grid = Grid::new(w, h);
    let chars: Vec<char> = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789 "
        .chars()
        .collect();
    for row in 0..h {
        for col in 0..w {
            let ch = chars[(row * w + col) % chars.len()];
            grid[(col, row)] = Cell::with_char(ch);
        }
    }
    grid
}

fn mixed_sgr_grid(w: usize, h: usize) -> Grid {
    let mut grid = Grid::new(w, h);
    let chars: Vec<char> = "ABCDEFGH中文字符测试ijklmn0123!@#$你好世界 "
        .chars()
        .collect();
    for row in 0..h {
        for col in 0..w {
            let ch = chars[(row * w + col) % chars.len()];
            let mut cell = Cell::with_char(ch);

            // Vary SGR attributes by position
            match col % 6 {
                0 => { /* default */ }
                1 => cell.flags |= CellFlags::BOLD,
                2 => {
                    cell.fg = Color::Indexed(1); // red
                }
                3 => {
                    cell.fg = Color::Rgb(100, 200, 50);
                    cell.flags |= CellFlags::ITALIC;
                }
                4 => cell.flags |= CellFlags::REVERSE,
                5 => {
                    cell.fg = Color::Indexed(4); // blue
                    cell.bg = Color::Rgb(40, 40, 40);
                }
                _ => {}
            }
            grid[(col, row)] = cell;
        }
    }
    grid
}

// ──── row_to_runs benchmarks ────

fn bench_row_to_runs(c: &mut Criterion) {
    let theme = RenderTheme::default();
    let cursor = CursorState::hidden();
    let sizes = [
        (80, 24, "80x24"),
        (200, 50, "200x50"),
        (500, 100, "500x100"),
    ];

    let mut group = c.benchmark_group("row_to_runs");

    // Empty grids
    for &(w, h, label) in &sizes {
        let grid = empty_grid(w, h);
        group.bench_with_input(BenchmarkId::new("empty", label), &grid, |b, g| {
            b.iter(|| {
                for row in 0..g.height() {
                    black_box(row_to_runs(
                        g,
                        row,
                        &theme,
                        Some(&cursor),
                        &[],
                        None,
                        None,
                        false,
                        &std::collections::HashMap::new(),
                    ));
                }
            })
        });
    }

    // ASCII-only grids
    for &(w, h, label) in &sizes {
        let grid = ascii_grid(w, h);
        group.bench_with_input(BenchmarkId::new("ascii", label), &grid, |b, g| {
            b.iter(|| {
                for row in 0..g.height() {
                    black_box(row_to_runs(
                        g,
                        row,
                        &theme,
                        Some(&cursor),
                        &[],
                        None,
                        None,
                        false,
                        &std::collections::HashMap::new(),
                    ));
                }
            })
        });
    }

    // Mixed SGR + CJK grids
    for &(w, h, label) in &sizes {
        let grid = mixed_sgr_grid(w, h);
        group.bench_with_input(BenchmarkId::new("mixed_sgr_cjk", label), &grid, |b, g| {
            b.iter(|| {
                for row in 0..g.height() {
                    black_box(row_to_runs(
                        g,
                        row,
                        &theme,
                        Some(&cursor),
                        &[],
                        None,
                        None,
                        false,
                        &std::collections::HashMap::new(),
                    ));
                }
            })
        });
    }

    group.finish();
}

// ──── row_to_text benchmarks ────

fn bench_row_to_text(c: &mut Criterion) {
    let sizes = [
        (80, 24, "80x24"),
        (200, 50, "200x50"),
        (500, 100, "500x100"),
    ];

    let mut group = c.benchmark_group("row_to_text");

    for &(w, h, label) in &sizes {
        let grid = ascii_grid(w, h);
        group.bench_with_input(BenchmarkId::new("ascii", label), &grid, |b, g| {
            b.iter(|| {
                for row in 0..g.height() {
                    black_box(row_to_text(g, row));
                }
            })
        });
    }

    for &(w, h, label) in &sizes {
        let grid = mixed_sgr_grid(w, h);
        group.bench_with_input(BenchmarkId::new("mixed", label), &grid, |b, g| {
            b.iter(|| {
                for row in 0..g.height() {
                    black_box(row_to_text(g, row));
                }
            })
        });
    }

    group.finish();
}

// ──── Color mapping benchmarks ────

fn bench_map_fg(c: &mut Criterion) {
    let theme = RenderTheme::default();
    let colors: Vec<Color> = (0..1000)
        .map(|i| match i % 4 {
            0 => Color::Default,
            1 => Color::Indexed((i % 256) as u8),
            2 => Color::Rgb((i % 256) as u8, (i * 7 % 256) as u8, (i * 13 % 256) as u8),
            _ => Color::Default,
        })
        .collect();

    c.bench_function("map_fg_1000", |b| {
        b.iter(|| {
            for &color in &colors {
                black_box(map_fg(color, &theme));
            }
        })
    });
}

fn bench_map_bg(c: &mut Criterion) {
    let theme = RenderTheme::default();
    let colors: Vec<Color> = (0..1000)
        .map(|i| match i % 4 {
            0 => Color::Default,
            1 => Color::Indexed((i % 256) as u8),
            2 => Color::Rgb((i % 256) as u8, (i * 7 % 256) as u8, (i * 13 % 256) as u8),
            _ => Color::Default,
        })
        .collect();

    c.bench_function("map_bg_1000", |b| {
        b.iter(|| {
            for &color in &colors {
                black_box(map_bg(color, &theme));
            }
        })
    });
}

fn bench_indexed_to_rgb(c: &mut Criterion) {
    c.bench_function("indexed_to_rgb_all_256", |b| {
        b.iter(|| {
            for idx in 0u8..=255 {
                black_box(indexed_to_rgb(idx));
            }
        })
    });
}

// ──── Per-row conversion (single row, repeated) ────

fn bench_single_row(c: &mut Criterion) {
    let theme = RenderTheme::default();
    let cursor = CursorState::new(10, 0);

    let mut group = c.benchmark_group("single_row");

    for &(w, _, label) in &[
        (80, 24, "80cols"),
        (200, 50, "200cols"),
        (500, 100, "500cols"),
    ] {
        // ASCII row
        let grid_ascii = ascii_grid(w, 1);
        group.bench_with_input(BenchmarkId::new("ascii", label), &grid_ascii, |b, g| {
            b.iter(|| {
                black_box(row_to_runs(
                    g,
                    0,
                    &theme,
                    Some(&cursor),
                    &[],
                    None,
                    None,
                    false,
                    &std::collections::HashMap::new(),
                ))
            })
        });

        // Mixed SGR row
        let grid_mixed = mixed_sgr_grid(w, 1);
        group.bench_with_input(BenchmarkId::new("mixed_sgr", label), &grid_mixed, |b, g| {
            b.iter(|| {
                black_box(row_to_runs(
                    g,
                    0,
                    &theme,
                    Some(&cursor),
                    &[],
                    None,
                    None,
                    false,
                    &std::collections::HashMap::new(),
                ))
            })
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_row_to_runs,
    bench_row_to_text,
    bench_map_fg,
    bench_map_bg,
    bench_indexed_to_rgb,
    bench_single_row,
);
criterion_main!(benches);
