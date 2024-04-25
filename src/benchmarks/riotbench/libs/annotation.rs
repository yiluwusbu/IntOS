use super::compact_string::CompactString;

pub struct Annotation {
    key: &'static str,
    anno: &'static str,
}

impl Annotation {
    pub const fn new(key: &'static str, anno: &'static str) -> Self {
        Self { key, anno }
    }

    pub fn annotate<const N: usize>(&self, key: &'static str, s: &mut CompactString<N>) -> bool {
        if self.key == key {
            s.append_str(self.anno)
        } else {
            false
        }
    }
}
