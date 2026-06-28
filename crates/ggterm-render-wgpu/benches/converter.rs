//! Benchmarks for Grid → TextRun conversion (CPU-only, no GPU needed).

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use ggterm_core::{Cell, CellFlags, Color, Grid};
use ggterm_render::theme::RenderTheme;
use ggterm_render::CursorState;
use ggterm_render_wgpu::colors::{map_bg, map_fg};
use ggterm_render_wgpu::converter::{row_to_runs, row_to_text};

// ── row_to_runs: SGR grouping efficiency ─────────────────────────────

fn bench_row_to_runs(c: &mut Criterion) {
    let mut group = c.benchmark_group("row_to_runs");
    let theme = RenderTheme::default();
    let cursor = CursorState::hidden();

    // 80 cols, plain text
    let mut grid_plain = Grid::new(80, 1);
    for (i, ch) in "Hello World! This is a benchmark test for the ggterm render-wgpu converter module.".chars().enumerate() {
        if i < 80 {
            grid_plain.put_char(i, 0, ch);
        }
    }
    group.bench_function("80col_plain", |b| {
        b.iter(|| black_box(row_to_runs(black_box(&grid_plain), 0, &theme, Some(&cursor))))
    });

    // 80 cols, mixed colors (every 10th cell colored)
    let mut grid_color = Grid::new(80, 1);
    for i in 0..80 {
        let mut cell = Cell::with_char((b'a' + (i % 26) as u8) as char);
        if i % 10 == 0 {
            cell.fg = Color::Rgb(0xFF, 0x00, 0x00);
        }
        grid_color[(i, 0)] = cell;
    }
    group.bench_function("80col_mixed_colors", |b| {
        b.iter(|| black_box(row_to_runs(black_box(&grid_color), 0, &theme, Some(&cursor))))
    });

    // 80 cols, all bold
    let mut grid_bold = Grid::new(80, 1);
    for i in 0..80 {
        let mut cell = Cell::with_char((b'a' + (i % 26) as u8) as char);
        cell.flags = CellFlags::BOLD;
        grid_bold[(i, 0)] = cell;
    }
    group.bench_function("80col_all_bold", |b| {
        b.iter(|| black_box(row_to_runs(black_box(&grid_bold), 0, &theme, Some(&cursor))))
    });

    // 200 cols, plain text
    let mut grid_200 = Grid::new(200, 1);
    for i in 0..200 {
        grid_200.put_char(i, 0, (b'a' + (i % 26) as u8) as char);
    }
    group.bench_function("200col_plain", |b| {
        b.iter(|| black_box(row_to_runs(black_box(&grid_200), 0, &theme, Some(&cursor))))
    });

    // 500 cols, empty
    let grid_500 = Grid::new(500, 1);
    group.bench_function("500col_empty", |b| {
        b.iter(|| black_box(row_to_runs(black_box(&grid_500), 0, &theme, Some(&cursor))))
    });

    group.finish();
}

// ── row_to_text: full-row text extraction ────────────────────────────

fn bench_row_to_text(c: &mut Criterion) {
    let mut grid = Grid::new(80, 1);
    for (i, ch) in "Hello World! This is a benchmark test for ggterm render-wgpu row_to_text.".chars().enumerate() {
        if i < 80 {
            grid.put_char(i, 0, ch);
        }
    }

    c.bench_function("row_to_text_80col", |b| {
        b.iter(|| black_box(row_to_text(black_box(&grid), 0)))
    });
}

// ── map_fg/map_bg: color mapping ─────────────────────────────────────

fn bench_color_mapping(c: &mut Criterion) {
    let theme = RenderTheme::default();

    let mut group = c.benchmark_group("color_mapping");

    group.bench_function("map_fg_default", |b| {
        b.iter(|| black_box(map_fg(black_box(Color::Default), &theme)))
    });

    group.bench_function("map_fg_rgb", |b| {
        b.iter(|| black_box(map_fg(black_box(Color::Rgb(128, 64, 32)), &theme)))
    });

    group.bench_function("map_fg_indexed_16", |b| {
        b.iter(|| black_box(map_fg(black_box(Color::Indexed(3)), &theme)))
    });

    group.bench_function("map_fg_indexed_256", |b| {
        b.iter(|| black_box(map_fg(black_box(Color::Indexed(200)), &theme)))
    });

    group.bench_function("map_bg_default", |b| {
        b.iter(|| black_box(map_bg(black_box(Color::Default), &theme)))
    });

    // Batch: 1000 color lookups (simulating one full row)
    group.bench_function("map_fg_batch_80", |b| {
        b.iter(|| {
            for _ in 0..80 {
                black_box(map_fg(black_box(Color::Indexed(3)), &theme));
            }
        })
    });

    group.finish();
}

// ── Full grid conversion: row_to_runs for every row ──────────────────

fn bench_full_grid_conversion(c: &mut Criterion) {
    let theme = RenderTheme::default();
    let cursor = CursorState::new(40, 12);

    let mut group = c.benchmark_group("full_grid");

    // 80x24 terminal — mixed content
    let mut grid_80x24 = Grid::new(80, 24);
    for row in 0..24 {
        for col in 0..80 {
            let ch = (b' ' + ((col + row) % 95) as u8) as char;
            let mut cell = Cell::with_char(ch);
            if col % 20 == 0 {
                cell.fg = Color::Indexed(((col / 20) % 16) as u8);
            }
            grid_80x24[(col, row)] = cell;
        }
    }
    group.bench_function("80x24_all_rows", |b| {
        b.iter(|| {
            for row in 0..24 {
                black_box(row_to_runs(black_box(&grid_80x24), row, &theme, Some(&cursor)));
            }
        })
    });

    // 80x24 empty
    let grid_80x24_empty = Grid::new(80, 24);
    group.bench_function("80x24_empty", |b| {
        b.iter(|| {
            for row in 0..24 {
                black_box(row_to_runs(black_box(&grid_80x24_empty), row, &theme, Some(&cursor)));
            }
        })
    });

    // 200x50 terminal
    let grid_200x50 = Grid::new(200, 50);
    group.bench_function("200x50_empty", |b| {
        b.iter(|| {
            for row in 0..50 {
                black_box(row_to_runs(black_box(&grid_200x50), row, &theme, Some(&cursor)));
            }
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_row_to_runs,
    bench_row_to_text,
    bench_color_mapping,
    bench_full_grid_conversion,
);
criterion_main!(benches);
