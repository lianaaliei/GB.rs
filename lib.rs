use wasm_bindgen::prelude::*;

const Z: u8 = 0x80;
const N: u8 = 0x40;
const H: u8 = 0x20;
const C: u8 = 0x10;

fn fl(z: bool, n: bool, h: bool, c: bool) -> u8 {
    (if z { Z } else { 0 }) | (if n { N } else { 0 }) | (if h { H } else { 0 }) | (if c { C } else { 0 })
}

#[wasm_bindgen]
pub struct Gb {
    rom: Vec<u8>,
    vram: Vec<u8>,
    eram: Vec<u8>,
    wram: Vec<u8>,
    oam: Vec<u8>,
    io: Vec<u8>,
    hram: Vec<u8>,
    ie: u8,
    r: [u8; 8], // B C D E H L (HL) A
    f: u8,
    sp: u16,
    pc: u16,
    ime: bool,
    ei: u8,
    halted: bool,
    mbc: u8,
    bank: usize,
    rbank: usize,
    ram_en: bool,
    div: u16,
    tacc: u32,
    lc: u32,
    wly: u8,
    stat_line: bool,
    cyc: u32,
    fr: bool,
    joy: u8, // bit0..7: right left up down A B select start
    fb: Vec<u8>,
    ser: Vec<u8>,
}

#[wasm_bindgen]
impl Gb {
    #[wasm_bindgen(constructor)]
    pub fn new(rom: &[u8]) -> Gb {
        let t = rom.get(0x147).copied().unwrap_or(0);
        let mut g = Gb {
            rom: rom.to_vec(),
            vram: vec![0; 0x2000],
            eram: vec![0; 0x8000],
            wram: vec![0; 0x2000],
            oam: vec![0; 0xA0],
            io: vec![0; 0x80],
            hram: vec![0; 0x80],
            ie: 0,
            r: [0x00, 0x13, 0x00, 0xD8, 0x01, 0x4D, 0, 0x01],
            f: 0xB0,
            sp: 0xFFFE,
            pc: 0x0100,
            ime: false,
            ei: 0,
            halted: false,
            mbc: if t == 0 { 0 } else { 1 },
            bank: 1,
            rbank: 0,
            ram_en: false,
            div: 0xABCC,
            tacc: 0,
            lc: 0,
            wly: 0,
            stat_line: false,
            cyc: 0,
            fr: false,
            joy: 0,
            fb: vec![255; 160 * 144 * 4],
            ser: vec![],
        };
        g.io[0x40] = 0x91;
        g.io[0x47] = 0xFC;
        g.io[0x48] = 0xFF;
        g.io[0x49] = 0xFF;
        g.io[0x0F] = 0xE1;
        g
    }

    /// Runs one video frame and returns the RGBA framebuffer (160x144).
    pub fn frame(&mut self) -> Vec<u8> {
        let mut t = 0u32;
        while t < 140_000 && !self.fr {
            let c = self.step();
            self.tick(c);
            t += c;
        }
        self.fr = false;
        self.fb.clone()
    }

    /// bit: 0 right, 1 left, 2 up, 3 down, 4 A, 5 B, 6 select, 7 start
    pub fn key(&mut self, bit: u8, down: bool) {
        if down { self.joy |= 1 << bit } else { self.joy &= !(1 << bit) }
    }

    /// Text sent over the serial port (Blargg test ROMs print here).
    pub fn serial(&mut self) -> String {
        String::from_utf8_lossy(&std::mem::take(&mut self.ser)).into_owned()
    }

    pub fn title(&self) -> String {
        let t: Vec<u8> = self.rom.get(0x134..0x143).unwrap_or(&[]).iter().copied().take_while(|&b| b != 0).collect();
        String::from_utf8_lossy(&t).into_owned()
    }
}

// ---------- memory ----------
impl Gb {
    fn rb(&self, a: u16) -> u8 {
        let u = a as usize;
        match a {
            0..=0x3FFF => self.rom.get(u).copied().unwrap_or(0xFF),
            0x4000..=0x7FFF => {
                let nb = (self.rom.len() / 0x4000).max(1);
                self.rom.get((self.bank % nb) * 0x4000 + u - 0x4000).copied().unwrap_or(0xFF)
            }
            0x8000..=0x9FFF => self.vram[u - 0x8000],
            0xA000..=0xBFFF => if self.ram_en { self.eram[self.rbank * 0x2000 + u - 0xA000] } else { 0xFF },
            0xC000..=0xDFFF => self.wram[u - 0xC000],
            0xE000..=0xFDFF => self.wram[u - 0xE000],
            0xFE00..=0xFE9F => self.oam[u - 0xFE00],
            0xFF00 => {
                let sel = self.io[0];
                let mut low = 0x0F;
                if sel & 0x10 == 0 { low &= !(self.joy & 0x0F) }
                if sel & 0x20 == 0 { low &= !(self.joy >> 4) }
                0xC0 | (sel & 0x30) | low
            }
            0xFF04 => (self.div >> 8) as u8,
            0xFF0F => self.io[0x0F] | 0xE0,
            0xFF00..=0xFF7F => self.io[u - 0xFF00],
            0xFF80..=0xFFFE => self.hram[u - 0xFF80],
            0xFFFF => self.ie,
            _ => 0xFF,
        }
    }

    fn wb(&mut self, a: u16, v: u8) {
        let u = a as usize;
        match a {
            0..=0x1FFF => self.ram_en = v & 15 == 10,
            0x2000..=0x3FFF => if self.mbc == 1 { self.bank = (self.bank & 0x60) | (v & 31).max(1) as usize },
            0x4000..=0x5FFF => if self.mbc == 1 {
                self.bank = (self.bank & 31) | ((v as usize & 3) << 5);
                self.rbank = v as usize & 3;
            },
            0x8000..=0x9FFF => self.vram[u - 0x8000] = v,
            0xA000..=0xBFFF => if self.ram_en { self.eram[self.rbank * 0x2000 + u - 0xA000] = v },
            0xC000..=0xDFFF => self.wram[u - 0xC000] = v,
            0xE000..=0xFDFF => self.wram[u - 0xE000] = v,
            0xFE00..=0xFE9F => self.oam[u - 0xFE00] = v,
            0xFF00..=0xFF7F => match a {
                0xFF04 => self.div = 0,
                0xFF44 => {}
                0xFF41 => self.io[0x41] = (v & 0x78) | (self.io[0x41] & 7),
                0xFF02 => {
                    if v == 0x81 { let b = self.io[1]; self.ser.push(b) }
                    self.io[2] = v;
                }
                0xFF46 => {
                    let s = (v as u16) << 8;
                    for i in 0..0xA0u16 {
                        let b = self.rb(s + i);
                        self.oam[i as usize] = b;
                    }
                }
                _ => self.io[u - 0xFF00] = v,
            },
            0xFF80..=0xFFFE => self.hram[u - 0xFF80] = v,
            0xFFFF => self.ie = v,
            _ => {}
        }
    }

    // access helpers that burn 4 cycles each (M-cycle accurate-ish timing)
    fn rd(&mut self, a: u16) -> u8 { self.cyc += 4; self.rb(a) }
    fn wr(&mut self, a: u16, v: u8) { self.cyc += 4; self.wb(a, v) }
    fn fetch(&mut self) -> u8 { let v = self.rd(self.pc); self.pc = self.pc.wrapping_add(1); v }
    fn f16(&mut self) -> u16 { let l = self.fetch() as u16; let h = self.fetch() as u16; h << 8 | l }
    fn push(&mut self, v: u16) {
        self.cyc += 4;
        self.sp = self.sp.wrapping_sub(1); let s = self.sp; self.wr(s, (v >> 8) as u8);
        self.sp = self.sp.wrapping_sub(1); let s = self.sp; self.wr(s, v as u8);
    }
    fn pop(&mut self) -> u16 {
        let s = self.sp; let l = self.rd(s) as u16;
        let s = s.wrapping_add(1); let h = self.rd(s) as u16;
        self.sp = s.wrapping_add(1);
        h << 8 | l
    }
}

// ---------- CPU ----------
impl Gb {
    fn hl(&self) -> u16 { (self.r[4] as u16) << 8 | self.r[5] as u16 }
    fn get16(&self, i: u8) -> u16 {
        match i & 3 {
            0 => (self.r[0] as u16) << 8 | self.r[1] as u16,
            1 => (self.r[2] as u16) << 8 | self.r[3] as u16,
            2 => self.hl(),
            _ => self.sp,
        }
    }
    fn set16(&mut self, i: u8, v: u16) {
        let (h, l) = ((v >> 8) as u8, v as u8);
        match i & 3 {
            0 => { self.r[0] = h; self.r[1] = l }
            1 => { self.r[2] = h; self.r[3] = l }
            2 => { self.r[4] = h; self.r[5] = l }
            _ => self.sp = v,
        }
    }
    fn get(&mut self, i: u8) -> u8 {
        if i == 6 { let a = self.hl(); self.rd(a) } else { self.r[i as usize] }
    }
    fn set(&mut self, i: u8, v: u8) {
        if i == 6 { let a = self.hl(); self.wr(a, v) } else { self.r[i as usize] = v }
    }
    fn cc(&self, i: u8) -> bool {
        match i & 3 { 0 => self.f & Z == 0, 1 => self.f & Z != 0, 2 => self.f & C == 0, _ => self.f & C != 0 }
    }

    fn alu(&mut self, k: u8, v: u8) {
        let a = self.r[7];
        let c = (self.f >> 4) & 1;
        match k {
            0 | 1 => {
                let cin = if k == 1 { c } else { 0 };
                let r = a as u16 + v as u16 + cin as u16;
                self.f = fl(r as u8 == 0, false, (a & 15) + (v & 15) + cin > 15, r > 255);
                self.r[7] = r as u8;
            }
            2 | 3 | 7 => {
                let cin = if k == 3 { c } else { 0 };
                let r = a as i16 - v as i16 - cin as i16;
                self.f = fl(r as u8 == 0, true, ((a & 15) as i16 - (v & 15) as i16 - cin as i16) < 0, r < 0);
                if k != 7 { self.r[7] = r as u8 }
            }
            4 => { let r = a & v; self.f = fl(r == 0, false, true, false); self.r[7] = r }
            5 => { let r = a ^ v; self.f = fl(r == 0, false, false, false); self.r[7] = r }
            _ => { let r = a | v; self.f = fl(r == 0, false, false, false); self.r[7] = r }
        }
    }

    fn cb(&mut self) {
        let op = self.fetch();
        let (r, y) = (op & 7, (op >> 3) & 7);
        let v = self.get(r);
        match op >> 6 {
            0 => {
                let cin = (self.f >> 4) & 1;
                let (res, c) = match y {
                    0 => (v << 1 | v >> 7, v >> 7),
                    1 => (v >> 1 | v << 7, v & 1),
                    2 => (v << 1 | cin, v >> 7),
                    3 => (v >> 1 | cin << 7, v & 1),
                    4 => (v << 1, v >> 7),
                    5 => (v >> 1 | v & 0x80, v & 1),
                    6 => (v << 4 | v >> 4, 0),
                    _ => (v >> 1, v & 1),
                };
                self.f = fl(res == 0, false, false, c == 1);
                self.set(r, res);
            }
            1 => self.f = (self.f & C) | fl((v >> y) & 1 == 0, false, true, false),
            2 => self.set(r, v & !(1 << y)),
            _ => self.set(r, v | (1 << y)),
        }
    }

    fn step(&mut self) -> u32 {
        self.cyc = 0;
        let pend = self.io[0x0F] & self.ie & 0x1F;
        if pend != 0 {
            self.halted = false;
            if self.ime {
                self.ime = false;
                let i = pend.trailing_zeros() as u16;
                self.io[0x0F] &= !(1 << i);
                let pc = self.pc;
                self.push(pc);
                self.pc = 0x40 + i * 8;
                self.cyc += 8;
                return self.cyc;
            }
        }
        if self.halted { return 4 }

        let op = self.fetch();
        match op {
            0x00 | 0x10 => {}
            0x76 => self.halted = true,
            0xCB => self.cb(),
            0xF3 => self.ime = false,
            0xFB => self.ei = 2,
            x if x & 0xCF == 0x01 => { let v = self.f16(); self.set16(x >> 4, v) }
            0x02 | 0x12 => { let a = self.get16((op >> 4) & 1); let v = self.r[7]; self.wr(a, v) }
            0x0A | 0x1A => { let a = self.get16((op >> 4) & 1); let v = self.rd(a); self.r[7] = v }
            0x22 | 0x32 => {
                let a = self.hl(); let v = self.r[7]; self.wr(a, v);
                self.set16(2, if op == 0x22 { a.wrapping_add(1) } else { a.wrapping_sub(1) });
            }
            0x2A | 0x3A => {
                let a = self.hl(); let v = self.rd(a); self.r[7] = v;
                self.set16(2, if op == 0x2A { a.wrapping_add(1) } else { a.wrapping_sub(1) });
            }
            x if x & 0xCF == 0x03 => { let i = x >> 4; let v = self.get16(i).wrapping_add(1); self.set16(i, v); self.cyc += 4 }
            x if x & 0xCF == 0x0B => { let i = x >> 4; let v = self.get16(i).wrapping_sub(1); self.set16(i, v); self.cyc += 4 }
            x if x & 0xCF == 0x09 => {
                let hl = self.hl(); let v = self.get16(x >> 4);
                self.f = (self.f & Z) | fl(false, false, (hl & 0xFFF) + (v & 0xFFF) > 0xFFF, hl as u32 + v as u32 > 0xFFFF);
                self.set16(2, hl.wrapping_add(v)); self.cyc += 4;
            }
            x if x & 0xC7 == 0x04 => {
                let i = (x >> 3) & 7; let v = self.get(i); let r = v.wrapping_add(1);
                self.f = (self.f & C) | fl(r == 0, false, v & 15 == 15, false); self.set(i, r);
            }
            x if x & 0xC7 == 0x05 => {
                let i = (x >> 3) & 7; let v = self.get(i); let r = v.wrapping_sub(1);
                self.f = (self.f & C) | fl(r == 0, true, v & 15 == 0, false); self.set(i, r);
            }
            x if x & 0xC7 == 0x06 => { let v = self.fetch(); self.set((x >> 3) & 7, v) }
            0x07 | 0x0F | 0x17 | 0x1F => {
                let a = self.r[7]; let cin = (self.f >> 4) & 1;
                let (res, c) = match op {
                    0x07 => (a << 1 | a >> 7, a >> 7),
                    0x0F => (a >> 1 | a << 7, a & 1),
                    0x17 => (a << 1 | cin, a >> 7),
                    _ => (a >> 1 | cin << 7, a & 1),
                };
                self.r[7] = res; self.f = fl(false, false, false, c == 1);
            }
            0x08 => { let a = self.f16(); let s = self.sp; self.wr(a, s as u8); self.wr(a.wrapping_add(1), (s >> 8) as u8) }
            0x18 | 0x20 | 0x28 | 0x30 | 0x38 => {
                let e = self.fetch() as i8 as u16;
                if op == 0x18 || self.cc((op >> 3) & 3) { self.pc = self.pc.wrapping_add(e); self.cyc += 4 }
            }
            0x27 => {
                let mut a = self.r[7]; let mut adj = 0u8; let mut c = self.f & C != 0;
                if self.f & H != 0 || (self.f & N == 0 && a & 15 > 9) { adj |= 6 }
                if c || (self.f & N == 0 && a > 0x99) { adj |= 0x60; c = true }
                a = if self.f & N != 0 { a.wrapping_sub(adj) } else { a.wrapping_add(adj) };
                self.r[7] = a; self.f = (self.f & N) | fl(a == 0, false, false, c);
            }
            0x2F => { self.r[7] = !self.r[7]; self.f = (self.f & (Z | C)) | N | H }
            0x37 => self.f = (self.f & Z) | C,
            0x3F => self.f = (self.f & Z) | (if self.f & C != 0 { 0 } else { C }),
            0x40..=0x7F => { let v = self.get(op & 7); self.set((op >> 3) & 7, v) }
            0x80..=0xBF => { let v = self.get(op & 7); self.alu((op >> 3) & 7, v) }
            0xE0 => { let n = self.fetch(); let v = self.r[7]; self.wr(0xFF00 | n as u16, v) }
            0xF0 => { let n = self.fetch(); let v = self.rd(0xFF00 | n as u16); self.r[7] = v }
            0xE2 => { let a = 0xFF00 | self.r[1] as u16; let v = self.r[7]; self.wr(a, v) }
            0xF2 => { let a = 0xFF00 | self.r[1] as u16; let v = self.rd(a); self.r[7] = v }
            0xEA => { let a = self.f16(); let v = self.r[7]; self.wr(a, v) }
            0xFA => { let a = self.f16(); let v = self.rd(a); self.r[7] = v }
            0xE8 | 0xF8 => {
                let e = self.fetch(); let sp = self.sp;
                let r = sp.wrapping_add(e as i8 as u16);
                self.f = fl(false, false, (sp & 15) + (e as u16 & 15) > 15, (sp & 0xFF) + e as u16 > 0xFF);
                if op == 0xE8 { self.sp = r; self.cyc += 8 } else { self.set16(2, r); self.cyc += 4 }
            }
            0xF9 => { self.sp = self.hl(); self.cyc += 4 }
            0xE9 => self.pc = self.hl(),
            0xC9 | 0xD9 => { self.pc = self.pop(); self.cyc += 4; if op == 0xD9 { self.ime = true } }
            0xC3 => { self.pc = self.f16(); self.cyc += 4 }
            0xCD => { let a = self.f16(); let pc = self.pc; self.push(pc); self.pc = a }
            x if x & 0xE7 == 0xC0 => { self.cyc += 4; if self.cc(x >> 3) { self.pc = self.pop(); self.cyc += 4 } }
            x if x & 0xE7 == 0xC2 => { let a = self.f16(); if self.cc(x >> 3) { self.pc = a; self.cyc += 4 } }
            x if x & 0xE7 == 0xC4 => {
                let a = self.f16();
                if self.cc(x >> 3) { let pc = self.pc; self.push(pc); self.pc = a }
            }
            x if x & 0xCF == 0xC1 => {
                let v = self.pop();
                if x == 0xF1 { self.r[7] = (v >> 8) as u8; self.f = v as u8 & 0xF0 } else { self.set16(x >> 4, v) }
            }
            x if x & 0xCF == 0xC5 => {
                let v = if x == 0xF5 { (self.r[7] as u16) << 8 | self.f as u16 } else { self.get16(x >> 4) };
                self.push(v);
            }
            x if x & 0xC7 == 0xC6 => { let v = self.fetch(); self.alu((x >> 3) & 7, v) }
            x if x & 0xC7 == 0xC7 => { let pc = self.pc; self.push(pc); self.pc = (x & 0x38) as u16 }
            _ => {} // illegal opcode
        }
        if self.ei > 0 { self.ei -= 1; if self.ei == 0 { self.ime = true } }
        self.cyc
    }
}

// ---------- timer + PPU ----------
impl Gb {
    fn tick(&mut self, c: u32) {
        self.div = self.div.wrapping_add(c as u16);
        let tac = self.io[7];
        if tac & 4 != 0 {
            self.tacc += c;
            let p = [1024u32, 16, 64, 256][(tac & 3) as usize];
            while self.tacc >= p {
                self.tacc -= p;
                let (t, o) = self.io[5].overflowing_add(1);
                self.io[5] = if o { self.io[6] } else { t };
                if o { self.io[0x0F] |= 4 }
            }
        }
        self.ppu(c);
    }

    fn ppu(&mut self, c: u32) {
        if self.io[0x40] & 0x80 == 0 {
            self.lc = 0; self.io[0x44] = 0; self.io[0x41] &= !3;
            return;
        }
        self.lc += c;
        let mut ly = self.io[0x44];
        if self.lc >= 456 {
            self.lc -= 456;
            ly = (ly + 1) % 154;
            self.io[0x44] = ly;
            if ly == 144 { self.io[0x0F] |= 1; self.fr = true }
            if ly < 144 { self.line(ly) }
        }
        let mode = if ly >= 144 { 1 } else if self.lc < 80 { 2 } else if self.lc < 252 { 3 } else { 0 };
        let stat = self.io[0x41];
        let coin = ly == self.io[0x45];
        let line = (coin && stat & 0x40 != 0) || (mode == 0 && stat & 8 != 0)
            || (mode == 1 && stat & 0x10 != 0) || (mode == 2 && stat & 0x20 != 0);
        if line && !self.stat_line { self.io[0x0F] |= 2 }
        self.stat_line = line;
        self.io[0x41] = (stat & 0x78) | 0x80 | ((coin as u8) << 2) | mode;
    }

    fn put(&mut self, x: usize, y: u8, shade: u8) {
        const P: [[u8; 3]; 4] = [[0xE0, 0xF8, 0xD0], [0x88, 0xC0, 0x70], [0x34, 0x68, 0x56], [0x08, 0x18, 0x20]];
        let o = (y as usize * 160 + x) * 4;
        self.fb[o..o + 3].copy_from_slice(&P[shade as usize]);
        self.fb[o + 3] = 255;
    }

    fn line(&mut self, ly: u8) {
        let lcdc = self.io[0x40];
        let (scy, scx, bgp) = (self.io[0x42] as i32, self.io[0x43] as i32, self.io[0x47]);
        let wx = self.io[0x4B] as i32 - 7;
        if ly == 0 { self.wly = 0 }
        let win = lcdc & 0x20 != 0 && ly >= self.io[0x4A] && wx < 160;
        let mut bg = [0u8; 160];
        for x in 0..160usize {
            let mut ci = 0;
            if lcdc & 1 != 0 {
                let (base, px, py) = if win && x as i32 >= wx {
                    (if lcdc & 0x40 != 0 { 0x1C00 } else { 0x1800 }, x as i32 - wx, self.wly as i32)
                } else {
                    (if lcdc & 8 != 0 { 0x1C00 } else { 0x1800 }, (x as i32 + scx) & 255, (ly as i32 + scy) & 255)
                };
                let tile = self.vram[base + (py as usize / 8) * 32 + px as usize / 8];
                let addr = (if lcdc & 0x10 != 0 { tile as i32 * 16 } else { 0x1000 + (tile as i8 as i32) * 16 }) as usize;
                let r = addr + (py as usize & 7) * 2;
                let bit = 7 - (px as usize & 7);
                ci = ((self.vram[r] >> bit) & 1) | (((self.vram[r + 1] >> bit) & 1) << 1);
            }
            bg[x] = ci;
            self.put(x, ly, (bgp >> (ci * 2)) & 3);
        }
        if win { self.wly += 1 }

        if lcdc & 2 != 0 {
            let h = if lcdc & 4 != 0 { 16 } else { 8 };
            let mut spr: Vec<usize> = (0..40)
                .filter(|&i| { let y = self.oam[i * 4] as i32 - 16; (ly as i32) >= y && (ly as i32) < y + h })
                .take(10).collect();
            spr.sort_by_key(|&i| (self.oam[i * 4 + 1], i));
            for &i in spr.iter().rev() {
                let y = self.oam[i * 4] as i32 - 16;
                let x = self.oam[i * 4 + 1] as i32 - 8;
                let (mut t, at) = (self.oam[i * 4 + 2], self.oam[i * 4 + 3]);
                let mut row = ly as i32 - y;
                if at & 0x40 != 0 { row = h - 1 - row }
                if h == 16 { t &= 0xFE }
                let a = t as usize * 16 + row as usize * 2;
                let pal = if at & 0x10 != 0 { self.io[0x49] } else { self.io[0x48] };
                for px in 0..8i32 {
                    let sx = x + px;
                    if sx < 0 || sx >= 160 { continue }
                    let bit = (if at & 0x20 != 0 { px } else { 7 - px }) as usize;
                    let ci = ((self.vram[a] >> bit) & 1) | (((self.vram[a + 1] >> bit) & 1) << 1);
                    if ci == 0 || (at & 0x80 != 0 && bg[sx as usize] != 0) { continue }
                    self.put(sx as usize, ly, (pal >> (ci * 2)) & 3);
                }
            }
        }
    }
}
