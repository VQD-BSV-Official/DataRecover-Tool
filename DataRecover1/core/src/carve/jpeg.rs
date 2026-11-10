use super::Src;

/// Duyệt marker tới EOI; bỏ qua thumbnail trong APPn nhờ nhảy theo length.
pub fn size(s: &impl Src, off: u64) -> Option<u64> {
    let mut buf = vec![0u8; 1 << 16];
    let mut p = off + 2;
    loop {
        let [ff, m] = s.rd::<2>(p)?;
        if ff != 0xFF { return None; }
        match m {
            0xD9 => return Some(p + 2 - off),
            0x00 => return None,
            0xFF => { p += 1; continue; }
            0x01 | 0xD0..=0xD8 => { p += 2; continue; }
            _ => {}
        }
        let l = u16::from_be_bytes(s.rd(p + 2)?) as u64;
        if l < 2 { return None; }
        p += 2 + l;
        if m != 0xDA { continue; }
        // sau SOS: dữ liệu entropy, tìm marker thật (bỏ FF00, RSTn, FFFF)
        loop {
            let n = s.read_at(p, &mut buf).ok()?;
            if n < 2 { return None; }
            let b = &buf[..n];
            let mut i = 0;
            let hit = loop {
                let Some(k) = b[i..n - 1].iter().position(|&x| x == 0xFF) else { break None };
                i += k;
                let c = b[i + 1];
                if c != 0 && c != 0xFF && !(0xD0..=0xD7).contains(&c) { break Some(i); }
                i += 1;
            };
            match hit {
                Some(i) => { p += i as u64; break; }
                None => p += (n - 1) as u64, // giữ lại byte cuối (có thể là FF)
            }
        }
    }
}
