// Copyright 2015, 2016 Ethcore (UK) Ltd.
// This file is part of Parity.

// Parity is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Parity is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Parity.  If not, see <http://www.gnu.org/licenses/>.

//! Binary representation of types

use util::bytes::Populatable;
use util::numbers::{U256, U512, H256, H2048, Address};
use std::mem;
use std::collections::{VecDeque, BTreeMap};
use std::ops::Range;
use super::Handshake;

#[derive(Debug)]
pub struct BinaryConvertError;

pub trait BinaryConvertable : Sized {
	fn size(&self) -> usize {
		mem::size_of::<Self>()
	}

	fn to_bytes(&self, buffer: &mut [u8], length_stack: &mut VecDeque<usize>) -> Result<(), BinaryConvertError>;

	fn from_bytes(buffer: &[u8], length_stack: &mut VecDeque<usize>) -> Result<Self, BinaryConvertError>;

	fn from_empty_bytes() -> Result<Self, BinaryConvertError> {
		Err(BinaryConvertError)
	}

	fn len_params() -> usize {
		0
	}
}

impl<T> BinaryConvertable for Option<T> where T: BinaryConvertable {
	fn size(&self) -> usize {
		match * self { None => 0, Some(ref val) => val.size() }
	}

	fn to_bytes(&self, buffer: &mut [u8], length_stack: &mut VecDeque<usize>) -> Result<(), BinaryConvertError> {
		match *self { None => Err(BinaryConvertError), Some(ref val) => val.to_bytes(buffer, length_stack) }
	}

	fn from_bytes(buffer: &[u8], length_stack: &mut VecDeque<usize>) -> Result<Self, BinaryConvertError> {
		if buffer.len() == 0 { return Self::from_empty_bytes(); }
		Ok(Some(try!(T::from_bytes(buffer, length_stack))))
	}

	fn from_empty_bytes() -> Result<Self, BinaryConvertError> {
		Ok(None)
	}

	fn len_params() -> usize {
		1
	}
}

impl<E: BinaryConvertable> BinaryConvertable for Result<(), E> {
	fn size(&self) -> usize {
		match *self {
			Ok(_) => 0,
			Err(ref e) => e.size(),
		}
	}

	fn to_bytes(&self, buffer: &mut [u8], length_stack: &mut VecDeque<usize>) -> Result<(), BinaryConvertError> {
		match *self {
			Ok(_) => Err(BinaryConvertError),
			Err(ref e) => Ok(try!(e.to_bytes(buffer, length_stack))),
		}
	}

	fn from_bytes(buffer: &[u8], length_stack: &mut VecDeque<usize>) -> Result<Self, BinaryConvertError> {
		Ok(Err(try!(E::from_bytes(&buffer, length_stack))))
	}

	fn from_empty_bytes() -> Result<Self, BinaryConvertError> {
		Ok(Ok(()))
	}

	fn len_params() -> usize {
		1
	}
}


impl<R: BinaryConvertable> BinaryConvertable for Result<R, ()> {
	fn size(&self) -> usize {
		match *self {
			Ok(ref r) => r.size(),
			Err(_) => 0,
		}
	}

	fn to_bytes(&self, buffer: &mut [u8], length_stack: &mut VecDeque<usize>) -> Result<(), BinaryConvertError> {
		match *self {
			Ok(ref r) => Ok(try!(r.to_bytes(buffer, length_stack))),
			Err(_) => Err(BinaryConvertError),
		}
	}

	fn from_bytes(buffer: &[u8], length_stack: &mut VecDeque<usize>) -> Result<Self, BinaryConvertError> {
		Ok(Ok(try!(R::from_bytes(&buffer, length_stack))))
	}

	fn from_empty_bytes() -> Result<Self, BinaryConvertError> {
		Ok(Err(()))
	}

	fn len_params() -> usize {
		1
	}
}

impl<R: BinaryConvertable, E: BinaryConvertable> BinaryConvertable for Result<R, E> {
	fn size(&self) -> usize {
		1usize + match *self {
			Ok(ref r) => r.size(),
			Err(ref e) => e.size(),
		}
	}

	fn to_bytes(&self, buffer: &mut [u8], length_stack: &mut VecDeque<usize>) -> Result<(), BinaryConvertError> {
		match *self {
			Ok(ref r) => {
				buffer[0] = 0;
				if r.size() > 0 {
					Ok(try!(r.to_bytes(&mut buffer[1..], length_stack)))
				}
				else { Ok(()) }
			},
			Err(ref e) => {
				buffer[0] = 1;
				if e.size() > 0 {
					Ok(try!(e.to_bytes(&mut buffer[1..], length_stack)))
				}
				else { Ok(()) }
			},
		}
	}

	fn from_bytes(buffer: &[u8], length_stack: &mut VecDeque<usize>) -> Result<Self, BinaryConvertError> {
		match buffer[0] {
			0 => {
				match buffer.len() {
					1 => Ok(Ok(try!(R::from_empty_bytes()))),
					_ => Ok(Ok(try!(R::from_bytes(&buffer[1..], length_stack)))),
				}
			}
			1 => Ok(Err(try!(E::from_bytes(&buffer[1..], length_stack)))),
			_ => Err(BinaryConvertError)
		}
	}

	fn len_params() -> usize {
		1
	}
}

impl<K, V> BinaryConvertable for BTreeMap<K, V> where K : BinaryConvertable + Ord, V: BinaryConvertable {
	fn size(&self) -> usize {
		0usize + match K::len_params() {
			0 => mem::size_of::<K>() * self.len(),
			_ => self.iter().fold(0usize, |acc, (k, _)| acc + k.size())
		} + match V::len_params() {
			0 => mem::size_of::<V>() * self.len(),
			_ => self.iter().fold(0usize, |acc, (_, v)| acc + v.size())
		}
	}

	fn to_bytes(&self, buffer: &mut [u8], length_stack: &mut VecDeque<usize>) -> Result<(), BinaryConvertError> {
		let mut offset = 0usize;
		for (key, val) in self.iter() {
			let key_size = match K::len_params() {
				0 => mem::size_of::<K>(),
				_ => { let size = key.size(); length_stack.push_back(size); size }
			};
			let val_size = match K::len_params() {
				0 => mem::size_of::<V>(),
				_ => { let size = val.size(); length_stack.push_back(size); size }
			};

			if key_size > 0 {
				let item_end = offset + key_size;
				try!(key.to_bytes(&mut buffer[offset..item_end], length_stack));
				offset = item_end;
			}

			if val_size > 0 {
				let item_end = offset + key_size;
				try!(val.to_bytes(&mut buffer[offset..item_end], length_stack));
				offset = item_end;
			}
		}
		Ok(())
	}

	fn from_bytes(buffer: &[u8], length_stack: &mut VecDeque<usize>) -> Result<Self, BinaryConvertError> {
		let mut index = 0;
		let mut result = Self::new();

		if buffer.len() == 0 { return Ok(result); }

		loop {
			let key_size = match K::len_params() {
				0 => mem::size_of::<K>(),
				_ => try!(length_stack.pop_front().ok_or(BinaryConvertError)),
			};
			let key = if key_size == 0 {
				try!(K::from_empty_bytes())
			} else {
				try!(K::from_bytes(&buffer[index..index+key_size], length_stack))
			};
			index = index + key_size;

			let val_size = match V::len_params() {
				0 => mem::size_of::<V>(),
				_ => try!(length_stack.pop_front().ok_or(BinaryConvertError)),
			};
			let val = if val_size == 0 {
				try!(V::from_empty_bytes())
			} else {
				try!(V::from_bytes(&buffer[index..index+val_size], length_stack))
			};
			result.insert(key, val);
			index = index + val_size;

			if index == buffer.len() { break; }
			if index > buffer.len() {
				return Err(BinaryConvertError)
			}
		}

		Ok(result)
	}

	fn from_empty_bytes() -> Result<Self, BinaryConvertError> {
		Ok(Self::new())
	}

	fn len_params() -> usize {
		1
	}
}

impl<T> BinaryConvertable for Vec<T> where T: BinaryConvertable {
	fn size(&self) -> usize {
		match T::len_params() {
			0 => mem::size_of::<T>() * self.len(),
			_ => self.iter().fold(0usize, |acc, t| acc + t.size()),
		}
	}

	fn to_bytes(&self, buffer: &mut [u8], length_stack: &mut VecDeque<usize>) -> Result<(), BinaryConvertError> {
		let mut offset = 0usize;
		for item in self.iter() {
			let next_size = match T::len_params() {
				0 => mem::size_of::<T>(),
				_ => { let size = item.size(); length_stack.push_back(size); size },
			};
			if next_size > 0 {
				let item_end = offset + next_size;
				try!(item.to_bytes(&mut buffer[offset..item_end], length_stack));
				offset = item_end;
			}
		}
		Ok(())
	}

	fn from_bytes(buffer: &[u8], length_stack: &mut VecDeque<usize>) -> Result<Self, BinaryConvertError> {
		let mut index = 0;
		let mut result = Self::with_capacity(
			match T::len_params() {
				0 => buffer.len() /  mem::size_of::<T>(),
				_ => 128,
			});

		if buffer.len() == 0 { return Ok(result); }

		loop {
			let next_size = match T::len_params() {
				0 => mem::size_of::<T>(),
				_ => try!(length_stack.pop_front().ok_or(BinaryConvertError)),
			};
			let item = if next_size == 0 {
				try!(T::from_empty_bytes())
			}
			else {
				try!(T::from_bytes(&buffer[index..index+next_size], length_stack))
			};
			result.push(item);

			index = index + next_size;
			if index == buffer.len() { break; }
			if index > buffer.len() {
				return Err(BinaryConvertError)
			}

		}

		Ok(result)
	}

	fn from_empty_bytes() -> Result<Self, BinaryConvertError> {
		Ok(Self::new())
	}

	fn len_params() -> usize {
		1
	}
}

impl BinaryConvertable for String {
	fn size(&self) -> usize {
		self.as_bytes().len()
	}

	fn from_empty_bytes() -> Result<Self, BinaryConvertError> {
		Ok(String::new())
	}

	fn to_bytes(&self, buffer: &mut [u8], _length_stack: &mut VecDeque<usize>) -> Result<(), BinaryConvertError> {
		buffer[..].clone_from_slice(self.as_bytes());
		Ok(())
	}

	fn from_bytes(buffer: &[u8], _length_stack: &mut VecDeque<usize>) -> Result<Self, BinaryConvertError> {
		Ok(::std::str::from_utf8(buffer).unwrap().to_owned())
	}

	fn len_params() -> usize {
		1
	}
}

impl<T> BinaryConvertable for Range<T> where T: BinaryConvertable {
	fn size(&self) -> usize {
		mem::size_of::<T>() * 2
	}

	fn from_empty_bytes() -> Result<Self, BinaryConvertError> {
		Err(BinaryConvertError)
	}

	fn to_bytes(&self, buffer: &mut[u8], length_stack: &mut VecDeque<usize>) -> Result<(), BinaryConvertError> {
		try!(self.start.to_bytes(&mut buffer[..mem::size_of::<T>()], length_stack));
		try!(self.end.to_bytes(&mut buffer[mem::size_of::<T>() + 1..], length_stack));
		Ok(())
	}

	fn from_bytes(buffer: &[u8], length_stack: &mut VecDeque<usize>) -> Result<Self, BinaryConvertError> {
		Ok(try!(T::from_bytes(&buffer[..mem::size_of::<T>()], length_stack))..try!(T::from_bytes(&buffer[mem::size_of::<T>()+1..], length_stack)))
	}

	fn len_params() -> usize {
		assert_eq!(0, T::len_params());
		0
	}
}

impl<T> BinaryConvertable for ::std::cell::RefCell<T> where T: BinaryConvertable {
	fn size(&self) -> usize {
		self.borrow().size()
	}

	fn from_empty_bytes() -> Result<Self, BinaryConvertError> {
		Ok(::std::cell::RefCell::new(try!(T::from_empty_bytes())))
	}

	fn from_bytes(buffer: &[u8], length_stack: &mut VecDeque<usize>) -> Result<Self, BinaryConvertError> {
		Ok(::std::cell::RefCell::new(try!(T::from_bytes(buffer, length_stack))))
	}

	fn to_bytes(&self, buffer: &mut [u8], length_stack: &mut VecDeque<usize>) -> Result<(), BinaryConvertError> {
		try!(self.borrow().to_bytes(buffer, length_stack));
		Ok(())
	}

	fn len_params() -> usize {
		T::len_params()
	}
}

impl<T> BinaryConvertable for ::std::cell::Cell<T> where T: BinaryConvertable + Copy {
	fn size(&self) -> usize {
		self.get().size()
	}

	fn from_empty_bytes() -> Result<Self, BinaryConvertError> {
		Ok(::std::cell::Cell::new(try!(T::from_empty_bytes())))
	}

	fn from_bytes(buffer: &[u8], length_stack: &mut VecDeque<usize>) -> Result<Self, BinaryConvertError> {
		Ok(::std::cell::Cell::new(try!(T::from_bytes(buffer, length_stack))))
	}

	fn to_bytes(&self, buffer: &mut [u8], length_stack: &mut VecDeque<usize>) -> Result<(), BinaryConvertError> {
		try!(self.get().to_bytes(buffer, length_stack));
		Ok(())
	}

	fn len_params() -> usize {
		T::len_params()
	}
}

impl BinaryConvertable for Vec<u8> {
	fn size(&self) -> usize {
		self.len()
	}

	fn from_empty_bytes() -> Result<Self, BinaryConvertError> {
		Ok(Vec::new())
	}

	fn to_bytes(&self, buffer: &mut [u8], _length_stack: &mut VecDeque<usize>) -> Result<(), BinaryConvertError> {
		buffer[..].clone_from_slice(&self[..]);
		Ok(())
	}

	fn from_bytes(buffer: &[u8], _length_stack: &mut VecDeque<usize>) -> Result<Self, BinaryConvertError> {
		let mut res = Self::with_capacity(buffer.len());
		unsafe { res.set_len(buffer.len()) }
		res[..].clone_from_slice(&buffer[..]);
		Ok(res)
	}

	fn len_params() -> usize {
		1
	}
}

pub fn deserialize_from<T, R>(r: &mut R) -> Result<T, BinaryConvertError>
	where R: ::std::io::Read,
		T: BinaryConvertable
{
	let mut fake_stack = VecDeque::new();

	match T::len_params() {
		0 => {
			let fixed_size = mem::size_of::<T>();
			let mut payload_buffer = Vec::with_capacity(fixed_size);
			unsafe { payload_buffer.set_len(fixed_size); }
			try!(r.read(&mut payload_buffer).map_err(|_| BinaryConvertError));
			T::from_bytes(&payload_buffer[..], &mut fake_stack)
		},
		_ => {
			let mut payload = Vec::new();
			try!(r.read_to_end(&mut payload).map_err(|_| BinaryConvertError));

			let stack_len = try!(u64::from_bytes(&payload[0..8], &mut fake_stack)) as usize;
			let mut length_stack = VecDeque::<usize>::with_capacity(stack_len);

			if stack_len > 0 {
				for idx in 0..stack_len {
					let stack_item = try!(u64::from_bytes(&payload[8 + idx*8..8 + (idx+1)*8], &mut fake_stack));
					length_stack.push_back(stack_item as usize);
				}
			}

			//try!(r.read(&mut size_buffer).map_err(|_| BinaryConvertError));
			let size = try!(u64::from_bytes(&payload[8+stack_len*8..16+stack_len*8], &mut fake_stack)) as usize;
			match size {
				0 => {
					T::from_empty_bytes()
				},
				_ => {
					T::from_bytes(&payload[16+stack_len*8..], &mut length_stack)
				}
			}
		},
	}
}

pub fn deserialize<T: BinaryConvertable>(buffer: &[u8]) -> Result<T, BinaryConvertError> {
	use std::io::Cursor;
	let mut buff = Cursor::new(buffer);
	deserialize_from::<T, _>(&mut buff)
}

pub fn serialize_into<T, W>(t: &T, w: &mut W) -> Result<(), BinaryConvertError>
	where W: ::std::io::Write,
		T: BinaryConvertable
{
	let mut fake_stack = VecDeque::new();

	match T::len_params() {
		0 => {
			let fixed_size = mem::size_of::<T>();
			let mut buffer = Vec::with_capacity(fixed_size);
			unsafe { buffer.set_len(fixed_size); }
			try!(t.to_bytes(&mut buffer[..], &mut fake_stack));
			try!(w.write(&buffer[..]).map_err(|_| BinaryConvertError));
			Ok(())
		},
		_ => {
			let mut length_stack = VecDeque::<usize>::new();
			let mut size_buffer = [0u8; 8];

			let size = t.size();
			if size == 0 {
				try!(w.write(&size_buffer).map_err(|_| BinaryConvertError));
				try!(w.write(&size_buffer).map_err(|_| BinaryConvertError));
				return Ok(());
			}

			let mut buffer = Vec::with_capacity(size);
			unsafe { buffer.set_len(size); }
			try!(t.to_bytes(&mut buffer[..], &mut length_stack));

			let stack_len = length_stack.len();
			try!((stack_len as u64).to_bytes(&mut size_buffer[..], &mut fake_stack));
			try!(w.write(&size_buffer[..]).map_err(|_| BinaryConvertError));
			if stack_len > 0 {
				let mut header_buffer = Vec::with_capacity(stack_len * 8);
				unsafe {  header_buffer.set_len(stack_len * 8); };
				try!((stack_len as u64).to_bytes(&mut header_buffer[0..8], &mut fake_stack));
				let mut idx = 0;
				loop {
					match length_stack.pop_front() {
						Some(val) => try!((val as u64).to_bytes(&mut header_buffer[idx * 8..(idx+1) * 8], &mut fake_stack)),
						None => { break; }
					}
					idx = idx + 1;
				}
				try!(w.write(&header_buffer[..]).map_err(|_| BinaryConvertError));
			}

			try!((size as u64).to_bytes(&mut size_buffer[..], &mut fake_stack));
			try!(w.write(&size_buffer[..]).map_err(|_| BinaryConvertError));

			try!(w.write(&buffer[..]).map_err(|_| BinaryConvertError));

			Ok(())
		},
	}
}

pub fn serialize<T: BinaryConvertable>(t: &T) -> Result<Vec<u8>, BinaryConvertError> {
	use std::io::Cursor;
	let mut buff = Cursor::new(Vec::new());
	try!(serialize_into(t, &mut buff));
	let into_inner = buff.into_inner();
	Ok(into_inner)
}

#[macro_export]
macro_rules! binary_fixed_size {
	($target_ty: ty) => {
		impl BinaryConvertable for $target_ty {
			fn from_bytes(bytes: &[u8], _length_stack: &mut VecDeque<usize>) -> Result<Self, BinaryConvertError> {
				match bytes.len().cmp(&::std::mem::size_of::<$target_ty>()) {
					::std::cmp::Ordering::Less => return Err(BinaryConvertError),
					::std::cmp::Ordering::Greater => return Err(BinaryConvertError),
					::std::cmp::Ordering::Equal => ()
				};
				let mut res: Self = unsafe { ::std::mem::uninitialized() };
				res.copy_raw(bytes);
				Ok(res)
			}

			fn to_bytes(&self, buffer: &mut [u8], _length_stack: &mut VecDeque<usize>) -> Result<(), BinaryConvertError> {
				let sz = ::std::mem::size_of::<$target_ty>();
				let ip: *const $target_ty = self;
				let ptr: *const u8 = ip as *const _;
				unsafe {
					::std::ptr::copy(ptr, buffer.as_mut_ptr(), sz);
				}
				Ok(())
			}
		}
	}
}

/// Fixed-sized version of Handshake struct
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinHandshake {
	api_version: BinVersion,
	protocol_version: BinVersion,
}

/// Shorten version of semver Version without `pre` and `build` information
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinVersion {
	pub major: u64,
	pub minor: u64,
	pub patch: u64,
}

impl From<Handshake> for BinHandshake {
	fn from(other: Handshake) -> Self {
		BinHandshake {
			api_version: BinVersion::from(other.api_version),
			protocol_version: BinVersion::from(other.protocol_version),
		}
	}
}

impl BinHandshake {
	pub fn to_semver(self) -> Handshake {
		Handshake {
			api_version: self.api_version.to_semver(),
			protocol_version: self.protocol_version.to_semver(),
		}
	}
}

impl BinVersion {
	pub fn to_semver(self) -> ::semver::Version {
		::semver::Version {
			major: self.major,
			minor: self.minor,
			patch: self.patch,
			pre: vec![],
			build: vec![],
		}
	}
}

impl From<::semver::Version> for BinVersion {
	fn from(other: ::semver::Version) -> Self {
 		BinVersion {
			major: other.major,
			minor: other.minor,
			patch: other.patch,
		}
	}
}

binary_fixed_size!(u16);
binary_fixed_size!(u64);
binary_fixed_size!(u32);
binary_fixed_size!(usize);
binary_fixed_size!(i32);
binary_fixed_size!(bool);
binary_fixed_size!(U256);
binary_fixed_size!(U512);
binary_fixed_size!(H256);
binary_fixed_size!(H2048);
binary_fixed_size!(Address);
binary_fixed_size!(BinHandshake);

#[test]
fn vec_serialize() {
	let mut v = Vec::new();
	v.push(5u64);
	v.push(10u64);
	let mut length_stack = VecDeque::new();
	let mut data = Vec::with_capacity(v.size());
	unsafe { data.set_len(v.size()); }
	let result = v.to_bytes(&mut data[..], &mut length_stack);

	assert!(result.is_ok());
	assert_eq!(5, data[0]);
	assert_eq!(0, data[1]);
	assert_eq!(10, data[8]);
	assert_eq!(0, data[12]);
}

#[test]
fn calculates_size() {
	let mut v = Vec::new();
	v.push(5u64);
	v.push(10u64);

	assert_eq!(16, v.size());
}

#[test]
fn vec_deserialize() {
	let data = [
		10u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8,
		5u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8,
	];

	let mut length_stack = VecDeque::new();
	let vec = Vec::<u64>::from_bytes(&data[..], &mut length_stack).unwrap();

	assert_eq!(vec![10u64, 5u64], vec);
}

#[test]
fn vec_deserialize_chained() {
	let mut v = Vec::new();
	v.push(Some(5u64));
	v.push(Some(10u64));
	v.push(None);
	v.push(Some(12u64));

	let mut length_stack = VecDeque::new();
	let mut data = Vec::with_capacity(v.size());
	unsafe { data.set_len(v.size()); }
	let result = v.to_bytes(&mut data[..], &mut length_stack);

	assert!(result.is_ok());
	assert_eq!(4, length_stack.len());
}

#[test]
fn vec_serialize_deserialize() {
	let mut v = Vec::new();
	v.push(Some(5u64));
	v.push(None);
	v.push(Some(10u64));
	v.push(None);
	v.push(Some(12u64));


	let mut data = Vec::with_capacity(v.size());
	unsafe { data.set_len(v.size()); }
	let mut length_stack = VecDeque::new();

	v.to_bytes(&mut data[..], &mut length_stack).unwrap();
	let de_v = Vec::<Option<u64>>::from_bytes(&data[..], &mut length_stack).unwrap();

	assert_eq!(v, de_v);
}

#[test]
fn serialize_into_ok() {
	use std::io::Cursor;
    let mut buff = Cursor::new(vec![0; 128]);

	let mut v = Vec::new();
	v.push(Some(5u64));
	v.push(None);
	v.push(Some(10u64));
	v.push(None);
	v.push(Some(12u64));

	serialize_into(&v, &mut buff).unwrap();
	assert_eq!(5, buff.get_ref()[0]);
	assert_eq!(8, buff.get_ref()[8]);
	assert_eq!(0, buff.get_ref()[16]);
	assert_eq!(8, buff.get_ref()[24]);
}

#[test]
fn deserialize_from_ok() {
	use std::io::Cursor;
    let mut buff = Cursor::new(vec![
		0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8,
		16u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8,
		10u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8,
		5u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8,
	]);

	let vec = deserialize_from::<Vec<u64>, _>(&mut buff).unwrap();

	assert_eq!(vec![10u64, 5u64], vec);
}

#[test]
fn serialize_into_deserialize_from() {
	use std::io::{Cursor, SeekFrom, Seek};

	let mut buff = Cursor::new(Vec::new());
	let mut v = Vec::new();
	v.push(Some(5u64));
	v.push(None);
	v.push(Some(10u64));
	v.push(None);
	v.push(Some(12u64));

	serialize_into(&v, &mut buff).unwrap();
	buff.seek(SeekFrom::Start(0)).unwrap();
	let de_v = deserialize_from::<Vec<Option<u64>>, _>(&mut buff).unwrap();
	assert_eq!(v, de_v);
}

#[test]
fn serialize_vec_str() {
	// empty
	let source = Vec::<String>::new();
	let serialized = serialize(&source).unwrap();
	let deserialized = deserialize::<Vec<String>>(&serialized).unwrap();

	assert_eq!(source, deserialized);

	// with few values
	let mut source = Vec::<String>::new();
	source.push("val1".to_owned());
	source.push("val2".to_owned());
	let serialized = serialize(&source).unwrap();
	let deserialized = deserialize::<Vec<String>>(&serialized).unwrap();

	assert_eq!(source, deserialized);
}

#[test]
fn serialize_opt_str() {
	// none
	let source: Option<String> = None;
	let serialized = serialize(&source).unwrap();
	let deserialized = deserialize::<Option<String>>(&serialized).unwrap();

	assert_eq!(source, deserialized);

	// value
	let source: Option<String> = Some("i have value".to_owned());
	let serialized = serialize(&source).unwrap();
	let deserialized = deserialize::<Option<String>>(&serialized).unwrap();

	assert_eq!(source, deserialized);
}

#[test]
fn serialize_opt_vec() {
 	use std::io::Cursor;

	let mut buff = Cursor::new(Vec::new());
	let optional_vec: Option<Vec<u8>> = None;
	serialize_into(&optional_vec, &mut buff).unwrap();

	assert_eq!(&vec![0u8; 16], buff.get_ref());
}

#[test]
fn serialize_opt_vec_payload() {
	let optional_vec: Option<Vec<u8>> = None;
	let payload = serialize(&optional_vec).unwrap();

	assert_eq!(vec![0u8;16], payload);
}

#[test]
fn deserialize_opt_vec() {
	use std::io::Cursor;
    let mut buff = Cursor::new(vec![0u8; 16]);

	let vec = deserialize_from::<Option<Vec<u8>>, _>(&mut buff).unwrap();

	assert!(vec.is_none());
}

#[test]
fn deserialize_simple_err() {
	use std::io::Cursor;
    let mut buff = Cursor::new(vec![0u8; 16]);

	let result = deserialize_from::<Result<(), u32>, _>(&mut buff).unwrap();

	assert!(result.is_ok());
}

#[test]
fn serialize_opt_vec_in_out() {
	use std::io::{Cursor, SeekFrom, Seek};

	let mut buff = Cursor::new(Vec::new());
	let optional_vec: Option<Vec<u8>> = None;
	serialize_into(&optional_vec, &mut buff).unwrap();

	buff.seek(SeekFrom::Start(0)).unwrap();
	let vec = deserialize_from::<Option<Vec<u8>>, _>(&mut buff).unwrap();

	assert!(vec.is_none());
}

#[test]
fn serialize_err_opt_vec_in_out() {
	use std::io::{Cursor, SeekFrom, Seek};

	let mut buff = Cursor::new(Vec::new());
	let optional_vec: Result<Option<Vec<u8>>, u32> = Ok(None);
	serialize_into(&optional_vec, &mut buff).unwrap();

	buff.seek(SeekFrom::Start(0)).unwrap();
	let vec = deserialize_from::<Result<Option<Vec<u8>>, u32>, _>(&mut buff).unwrap();

	assert!(vec.is_ok());
}

#[test]
fn serialize_btree() {
	use std::io::{Cursor, SeekFrom, Seek};

	let mut buff = Cursor::new(Vec::new());
	let mut btree = BTreeMap::new();
	btree.insert(1u64, 5u64);
	serialize_into(&btree, &mut buff).unwrap();

	buff.seek(SeekFrom::Start(0)).unwrap();
	let res = deserialize_from::<BTreeMap<u64, u64>, _>(&mut buff).unwrap();

	assert_eq!(res[&1u64], 5u64);
}

#[test]
fn serialize_handshake() {
	use std::io::{Cursor, SeekFrom, Seek};

	let mut buff = Cursor::new(Vec::new());

	let handshake = Handshake {
		api_version: ::semver::Version::parse("1.2.0").unwrap(),
		protocol_version: ::semver::Version::parse("1.2.0").unwrap(),
	};

	serialize_into(&BinHandshake::from(handshake.clone()), &mut buff).unwrap();

	buff.seek(SeekFrom::Start(0)).unwrap();
	let res = deserialize_from::<BinHandshake, _>(&mut buff).unwrap().to_semver();

	assert_eq!(res, handshake);

}
