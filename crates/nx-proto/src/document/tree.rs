//! A MessagePack tree that remembers how each value was encoded. The preset document is not
//! canonical MessagePack (zeros written as int16, the section table as str16 of 48 bytes), so a
//! general codec that picks the shortest form cannot write it back byte for byte.

use super::DocError;

/// Encoding of an integer, as read; `Fix` is a positive or negative fixint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntForm {
    Fix,
    U8,
    U16,
    U32,
    U64,
    I8,
    I16,
    I32,
    I64,
}

/// Encoding of a length prefix (strings, binaries, arrays, maps).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LenForm {
    Fix,
    L8,
    L16,
    L32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Nil,
    Bool(bool),
    Int(i64, IntForm),
    F32(f32),
    F64(f64),
    Str(Vec<u8>, LenForm),
    Bin(Vec<u8>, LenForm),
    Array(Vec<Item>, LenForm),
    Map(Vec<(Item, Item)>, LenForm),
}

impl Item {
    /// An integer in the shortest form, as a new value gets written.
    pub fn int(v: i64) -> Self {
        Self::Int(v, shortest_int(v))
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            Self::Int(v, _) => Some(*v),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Item]> {
        match self {
            Self::Array(a, _) => Some(a),
            _ => None,
        }
    }

    pub fn as_str_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Str(s, _) => Some(s),
            _ => None,
        }
    }

    pub fn as_map(&self) -> Option<&[(Item, Item)]> {
        match self {
            Self::Map(m, _) => Some(m),
            _ => None,
        }
    }

    /// The value under integer key `key` of a map.
    pub fn get(&self, key: i64) -> Option<&Item> {
        self.as_map()?.iter().find(|(k, _)| k.as_int() == Some(key)).map(|(_, v)| v)
    }

    pub fn get_mut(&mut self, key: i64) -> Option<&mut Item> {
        match self {
            Self::Map(m, _) => m.iter_mut().find(|(k, _)| k.as_int() == Some(key)).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn index_mut(&mut self, i: usize) -> Option<&mut Item> {
        match self {
            Self::Array(a, _) => a.get_mut(i),
            _ => None,
        }
    }

    pub fn write(&self, out: &mut Vec<u8>) {
        match self {
            Self::Nil => out.push(0xc0),
            Self::Bool(b) => out.push(if *b { 0xc3 } else { 0xc2 }),
            Self::Int(v, form) => write_int(out, *v, *form),
            Self::F32(f) => {
                out.push(0xca);
                out.extend_from_slice(&f.to_be_bytes());
            }
            Self::F64(f) => {
                out.push(0xcb);
                out.extend_from_slice(&f.to_be_bytes());
            }
            Self::Str(s, form) => {
                write_len(out, s.len(), *form, [Some(0xa0), Some(0xd9), Some(0xda), Some(0xdb)], 31);
                out.extend_from_slice(s);
            }
            Self::Bin(b, form) => {
                write_len(out, b.len(), *form, [None, Some(0xc4), Some(0xc5), Some(0xc6)], 0);
                out.extend_from_slice(b);
            }
            Self::Array(a, form) => {
                write_len(out, a.len(), *form, [Some(0x90), None, Some(0xdc), Some(0xdd)], 15);
                for item in a {
                    item.write(out);
                }
            }
            Self::Map(m, form) => {
                write_map_head(out, m.len(), *form);
                for (k, v) in m {
                    k.write(out);
                    v.write(out);
                }
            }
        }
    }
}

pub fn write_map_head(out: &mut Vec<u8>, len: usize, form: LenForm) {
    write_len(out, len, form, [Some(0x80), None, Some(0xde), Some(0xdf)], 15);
}

fn shortest_int(v: i64) -> IntForm {
    match v {
        -32..=127 => IntForm::Fix,
        128..=0xff => IntForm::U8,
        0x100..=0xffff => IntForm::U16,
        0x1_0000..=0xffff_ffff => IntForm::U32,
        0x1_0000_0000.. => IntForm::U64,
        -0x80..=-33 => IntForm::I8,
        -0x8000..=-0x81 => IntForm::I16,
        -0x8000_0000..=-0x8001 => IntForm::I32,
        _ => IntForm::I64,
    }
}

pub(crate) fn fits(v: i64, form: IntForm) -> bool {
    match form {
        IntForm::Fix => (-32..=127).contains(&v),
        IntForm::U8 => (0..=0xff).contains(&v),
        IntForm::U16 => (0..=0xffff).contains(&v),
        IntForm::U32 => (0..=0xffff_ffff).contains(&v),
        IntForm::U64 => v >= 0,
        IntForm::I8 => i8::try_from(v).is_ok(),
        IntForm::I16 => i16::try_from(v).is_ok(),
        IntForm::I32 => i32::try_from(v).is_ok(),
        IntForm::I64 => true,
    }
}

fn write_int(out: &mut Vec<u8>, v: i64, form: IntForm) {
    // An edited value that no longer fits its original form falls back to the shortest one.
    let form = if fits(v, form) { form } else { shortest_int(v) };
    match form {
        IntForm::Fix => out.push(v as u8),
        IntForm::U8 => out.extend([0xcc, v as u8]),
        IntForm::U16 => {
            out.push(0xcd);
            out.extend_from_slice(&(v as u16).to_be_bytes());
        }
        IntForm::U32 => {
            out.push(0xce);
            out.extend_from_slice(&(v as u32).to_be_bytes());
        }
        IntForm::U64 => {
            out.push(0xcf);
            out.extend_from_slice(&(v as u64).to_be_bytes());
        }
        IntForm::I8 => out.extend([0xd0, v as u8]),
        IntForm::I16 => {
            out.push(0xd1);
            out.extend_from_slice(&(v as i16).to_be_bytes());
        }
        IntForm::I32 => {
            out.push(0xd2);
            out.extend_from_slice(&(v as i32).to_be_bytes());
        }
        IntForm::I64 => {
            out.push(0xd3);
            out.extend_from_slice(&v.to_be_bytes());
        }
    }
}

/// `markers` are the fix base and the 8/16/32-bit markers; `None` where the type has no such form.
fn write_len(out: &mut Vec<u8>, len: usize, form: LenForm, markers: [Option<u8>; 4], fix_max: usize) {
    let fit = |f: LenForm| match f {
        LenForm::Fix => markers[0].is_some() && len <= fix_max,
        LenForm::L8 => markers[1].is_some() && len <= 0xff,
        LenForm::L16 => markers[2].is_some() && len <= 0xffff,
        LenForm::L32 => true,
    };
    let form = if fit(form) {
        form
    } else {
        [LenForm::Fix, LenForm::L8, LenForm::L16, LenForm::L32].into_iter().find(|f| fit(*f)).unwrap()
    };
    match form {
        LenForm::Fix => out.push(markers[0].unwrap() | len as u8),
        LenForm::L8 => out.extend([markers[1].unwrap(), len as u8]),
        LenForm::L16 => {
            out.push(markers[2].unwrap());
            out.extend_from_slice(&(len as u16).to_be_bytes());
        }
        LenForm::L32 => {
            out.push(markers[3].unwrap());
            out.extend_from_slice(&(len as u32).to_be_bytes());
        }
    }
}

pub struct Reader<'a> {
    buf: &'a [u8],
    pub pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub fn at_end(&self) -> bool {
        self.pos == self.buf.len()
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], DocError> {
        let end =
            self.pos.checked_add(n).filter(|e| *e <= self.buf.len()).ok_or(DocError::Truncated { offset: self.pos })?;
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn be(&mut self, n: usize) -> Result<u64, DocError> {
        Ok(self.take(n)?.iter().fold(0u64, |acc, b| acc << 8 | *b as u64))
    }

    pub fn item(&mut self) -> Result<Item, DocError> {
        let at = self.pos;
        let marker = self.take(1)?[0];
        let (form, n) = match marker {
            0x00..=0x7f => return Ok(Item::Int(marker as i64, IntForm::Fix)),
            0xe0..=0xff => return Ok(Item::Int(marker as i8 as i64, IntForm::Fix)),
            0xc0 => return Ok(Item::Nil),
            0xc2 => return Ok(Item::Bool(false)),
            0xc3 => return Ok(Item::Bool(true)),
            0xca => return Ok(Item::F32(f32::from_bits(self.be(4)? as u32))),
            0xcb => return Ok(Item::F64(f64::from_bits(self.be(8)?))),
            0xcc => return Ok(Item::Int(self.be(1)? as i64, IntForm::U8)),
            0xcd => return Ok(Item::Int(self.be(2)? as i64, IntForm::U16)),
            0xce => return Ok(Item::Int(self.be(4)? as i64, IntForm::U32)),
            0xcf => {
                let v = self.be(8)?;
                let v = i64::try_from(v)
                    .map_err(|_| DocError::Unsupported { offset: at, what: "uint64 above i64::MAX" })?;
                return Ok(Item::Int(v, IntForm::U64));
            }
            0xd0 => return Ok(Item::Int(self.be(1)? as u8 as i8 as i64, IntForm::I8)),
            0xd1 => return Ok(Item::Int(self.be(2)? as u16 as i16 as i64, IntForm::I16)),
            0xd2 => return Ok(Item::Int(self.be(4)? as u32 as i32 as i64, IntForm::I32)),
            0xd3 => return Ok(Item::Int(self.be(8)? as i64, IntForm::I64)),
            0xa0..=0xbf => (LenForm::Fix, (marker & 0x1f) as usize),
            0x80..=0x9f => (LenForm::Fix, (marker & 0x0f) as usize),
            0xc4 | 0xd9 => (LenForm::L8, self.be(1)? as usize),
            0xc5 | 0xda | 0xdc | 0xde => (LenForm::L16, self.be(2)? as usize),
            0xc6 | 0xdb | 0xdd | 0xdf => (LenForm::L32, self.be(4)? as usize),
            _ => return Err(DocError::Unsupported { offset: at, what: "MessagePack ext or reserved marker" }),
        };
        Ok(match marker {
            0xa0..=0xbf | 0xd9..=0xdb => Item::Str(self.take(n)?.to_vec(), form),
            0xc4..=0xc6 => Item::Bin(self.take(n)?.to_vec(), form),
            0x90..=0x9f | 0xdc | 0xdd => {
                // Every element takes at least one byte, which bounds a hostile length.
                if n > self.buf.len() - self.pos {
                    return Err(DocError::Truncated { offset: at });
                }
                Item::Array((0..n).map(|_| self.item()).collect::<Result<_, _>>()?, form)
            }
            _ => {
                if n > (self.buf.len() - self.pos) / 2 {
                    return Err(DocError::Truncated { offset: at });
                }
                let mut m = Vec::with_capacity(n);
                for _ in 0..n {
                    let k = self.item()?;
                    m.push((k, self.item()?));
                }
                Item::Map(m, form)
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(bytes: &[u8]) -> Item {
        let mut r = Reader::new(bytes);
        let item = r.item().unwrap();
        assert!(r.at_end());
        let mut out = Vec::new();
        item.write(&mut out);
        assert_eq!(out, bytes);
        item
    }

    #[test]
    fn keeps_non_canonical_forms() {
        assert_eq!(roundtrip(&[0xd1, 0, 0]), Item::Int(0, IntForm::I16));
        assert_eq!(roundtrip(&[0xcc, 0]), Item::Int(0, IntForm::U8));
        assert_eq!(roundtrip(&[0xda, 0, 2, b'h', b'i']), Item::Str(b"hi".to_vec(), LenForm::L16));
        assert_eq!(roundtrip(&[0xdc, 0, 1, 0xc0]), Item::Array(vec![Item::Nil], LenForm::L16));
        assert_eq!(roundtrip(&[0xff]), Item::Int(-1, IntForm::Fix));
        assert_eq!(roundtrip(&[0xd0, 0x80]), Item::Int(-128, IntForm::I8));
    }

    #[test]
    fn values_that_outgrow_their_form_are_widened() {
        let mut out = Vec::new();
        Item::Int(300, IntForm::Fix).write(&mut out);
        assert_eq!(out, [0xcd, 1, 0x2c]);
        out.clear();
        Item::Str(vec![b'x'; 40], LenForm::Fix).write(&mut out);
        assert_eq!(&out[..2], [0xd9, 40]);
    }

    #[test]
    fn shortest_forms_for_new_values() {
        assert_eq!(Item::int(127), Item::Int(127, IntForm::Fix));
        assert_eq!(Item::int(545), Item::Int(545, IntForm::U16));
        assert_eq!(Item::int(-33), Item::Int(-33, IntForm::I8));
    }

    #[test]
    fn truncated_input_is_an_error() {
        assert!(Reader::new(&[0xdc, 0xff, 0xff]).item().is_err());
        assert!(Reader::new(&[0xca, 0]).item().is_err());
        assert!(Reader::new(&[0xc1]).item().is_err());
    }
}
