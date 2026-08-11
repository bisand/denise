//! Prints glyphs as ASCII so the font can be reviewed without a display.
use denise_render::font;
fn main() {
    for line in std::env::args().skip(1) {
        let mut rows = vec![String::new(); font::CELL_HEIGHT as usize];
        for ch in line.chars() {
            let g = font::BUILT_IN.glyph(ch);
            for (i, row) in rows.iter_mut().enumerate() {
                for x in 0..font::CELL_WIDTH {
                    row.push(if g[i] & (0x80 >> x) != 0 { '#' } else { '.' });
                }
                row.push('.');
            }
        }
        for r in rows {
            println!("{r}");
        }
        println!();
    }
}
