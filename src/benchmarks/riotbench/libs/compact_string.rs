#[derive(Clone, Copy)]
pub struct CompactString<const N: usize> {
    str: [u8; N],
    len: usize,
}

impl<const N: usize> CompactString<N> {
    pub fn new() -> Self {
        Self {
            str: [0; N],
            len: 0,
        }
    }

    pub fn new_with(s: &str) -> Self {
        let mut cs = Self {
            str: [0; N],
            len: 0,
        };
        cs.append_str(s);
        return cs;
    }

    pub fn at(&self, i: usize) -> u8 {
        if i < self.len {
            self.str[i]
        } else {
            0
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn append<const M: usize>(&mut self, s: &CompactString<M>) -> bool {
        if s.len() + self.len > N {
            return false;
        }
        let mut j = self.len;
        for i in 0..s.len() {
            self.str[j] = s.at(i);
            j += 1;
        }
        self.len += s.len;
        return true;
    }

    pub fn append_str(&mut self, s: &str) -> bool {
        if s.len() + self.len > N {
            return false;
        }
        let mut j = self.len;
        for i in 0..s.len() {
            self.str[j] = s.as_bytes()[i];
            j += 1;
        }
        self.len += s.len();
        return true;
    }
}
