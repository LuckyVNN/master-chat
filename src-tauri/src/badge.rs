//! The unread count drawn as a red dot for the taskbar button's overlay.

const SIZE: usize = 32;

/// 3x5 pixel glyphs: 0-9 and '+'; each row is 3 bits, most significant left.
const GLYPHS: [[u8; 5]; 11] = [
    [0b111, 0b101, 0b101, 0b101, 0b111],
    [0b010, 0b110, 0b010, 0b010, 0b111],
    [0b111, 0b001, 0b111, 0b100, 0b111],
    [0b111, 0b001, 0b111, 0b001, 0b111],
    [0b101, 0b101, 0b111, 0b001, 0b001],
    [0b111, 0b100, 0b111, 0b001, 0b111],
    [0b111, 0b100, 0b111, 0b101, 0b111],
    [0b111, 0b001, 0b010, 0b010, 0b010],
    [0b111, 0b101, 0b111, 0b101, 0b111],
    [0b111, 0b101, 0b111, 0b001, 0b111],
    [0b000, 0b010, 0b111, 0b010, 0b000],
];

/// RGBA pixels, SIZE x SIZE.
pub fn render(count: u32) -> (Vec<u8>, u32, u32) {
    let mut px = vec![0u8; SIZE * SIZE * 4];
    let c = SIZE as f32 / 2.0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let d = ((x as f32 + 0.5 - c).powi(2) + (y as f32 + 0.5 - c).powi(2)).sqrt();
            // Antialiased edge.
            let alpha = (c - d + 0.5).clamp(0.0, 1.0);
            let i = (y * SIZE + x) * 4;
            px[i..i + 4].copy_from_slice(&[0xe5, 0x39, 0x35, (alpha * 255.0) as u8]);
        }
    }
    let glyphs: Vec<usize> = if count > 9 { vec![9, 10] } else { vec![count as usize] };
    let scale = if glyphs.len() == 1 { 4 } else { 3 };
    let gap = scale;
    let width = glyphs.len() * 3 * scale + (glyphs.len() - 1) * gap;
    let (left, top) = ((SIZE - width) / 2, (SIZE - 5 * scale) / 2);
    for (n, &g) in glyphs.iter().enumerate() {
        let gx = left + n * (3 * scale + gap);
        for (row, bits) in GLYPHS[g].iter().enumerate() {
            for col in 0..3 {
                if bits & (0b100 >> col) == 0 {
                    continue;
                }
                for dy in 0..scale {
                    for dx in 0..scale {
                        let i = ((top + row * scale + dy) * SIZE + gx + col * scale + dx) * 4;
                        px[i..i + 4].copy_from_slice(&[0xff, 0xff, 0xff, 0xff]);
                    }
                }
            }
        }
    }
    (px, SIZE as u32, SIZE as u32)
}
