use std::fmt;

#[derive(Debug)]
pub struct Trie {
	value: String,
	bits: [u128; 2],
	children: Vec<Trie>,
}

impl Trie {
	pub fn new() -> Self {
		Self {
			value: String::new(),
			bits: [0; _],
			children: vec![],
		}
	}

	fn has_key(&self, key: u8) -> bool {
		let bit_id = 1u128 << (key & 0x7F);
		self.bits[key as usize >> 7] & bit_id != 0
	}

	fn set_key_bit(&mut self, key: u8) {
		let bit_id = 1u128 << (key & 0x7F);
		self.bits[key as usize >> 7] |= bit_id;
	}

	fn index(&self, key: u8) -> usize {
		let shift = key & 0x7F;
		let mask = !(u128::MAX << shift);
		let idx = key as usize >> 7;
		let bits = self.bits[idx] | (1 << shift);
		let base: u32 = self.bits[..idx].iter().map(|&b| b.count_ones()).sum();
		(base + (bits & mask).count_ones()) as usize
	}

	pub fn insert(&mut self, word: String) {
		let mut n = self;
		for key in word.bytes() {
			let i = n.index(key);
			if n.has_key(key) {
				n = &mut n.children[i];
			} else {
				n.set_key_bit(key);
				n = n.children.insert_mut(i, Self::new());
			}
		}
		n.value = word
	}

	pub fn contains(&self, word: &str) -> bool {
		let mut n = self;
		for key in word.bytes() {
			if !n.has_key(key) {
				return false;
			}
			n = &n.children[n.index(key)];
		}
		n.value == word
	}

	fn collect_into<'a>(&'a self, dst: &mut Vec<&'a str>) {
		if !self.value.is_empty() {
			dst.push(self.value.as_str());
		}
		for n in &self.children {
			n.collect_into(dst)
		}
	}

	pub fn complete<'a>(&'a self, word: &str) -> Vec<&'a str> {
		let mut n = self;
		let mut v = vec![];

		for key in word.bytes() {
			if !n.has_key(key) {
				return vec![];
			}
			n = &n.children[n.index(key)];
		}

		n.collect_into(&mut v);
		v
	}
}

#[test]
fn test() {
	let mut t = Trie::new();
	t.insert("word".into());
	assert!(t.contains("word"));
	assert!(t.complete("wo").contains(&"word"));
}
