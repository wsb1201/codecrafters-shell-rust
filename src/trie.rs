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

	pub fn value(&self) -> Option<&str> {
		(!self.value.is_empty()).then_some(self.value.as_str())
	}

	pub fn complete_minimal<'a>(&'a self, word: &str) -> Option<&'a Trie> {
		let mut n = self;
		for key in word.bytes() {
			if !n.has_key(key) {
				return None;
			}
			n = &n.children[n.index(key)];
		}
		while let [single] = n.children.as_slice() {
			if single.children.len() < 2 {
				n = single;
			} else {
				break;
			}
		}
		Some(n)
	}

	pub fn is_leaf(&self) -> bool {
		self.children.is_empty()
	}

	pub fn collect_values<'a>(&'a self) -> Vec<&'a str> {
		fn collect_into<'a>(n: &'a Trie, dst: &mut Vec<&'a str>) {
			if let Some(val) = n.value() {
				dst.push(val);
			}
			for n in &n.children {
				collect_into(n, dst)
			}
		}

		let mut v = vec![];
		collect_into(self, &mut v);
		v
	}

	pub fn complete<'a>(&'a self, word: &str) -> Vec<&'a str> {
		let Some(n) = self.complete_minimal(word) else {
			return vec![];
		};
		n.collect_values()
	}
}

#[test]
fn test() {
	let mut t = Trie::new();
	t.insert("word".into());
	assert!(t.contains("word"));
	assert!(t.complete("wo").contains(&"word"));
}
