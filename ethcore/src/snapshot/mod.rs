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

//! Snapshot creation helpers.

use std::collections::VecDeque;
use std::fs::File;
use std::io::Write;
use std::path::Path;

use account_db::AccountDB;
use client::BlockChainClient;
use error::Error;
use ids::BlockID;
use views::BlockView;

use util::{Bytes, Hashable, HashDB, TrieDB};
use util::hash::{FixedHash, H256};
use util::numbers::U256;
use util::rlp::{DecoderError, Rlp, RlpStream, Stream, SHA3_NULL_RLP, UntrustedRlp, View};
use util::snappy;

use self::block::AbridgedBlock;

mod block;

// Try to have chunks be around 16MB (before compression)
const PREFERRED_CHUNK_SIZE: usize = 16 * 1024 * 1024;

// use initially 20MB for the reusable snappy buffers.
// should always be larger than PREFERRED_CHUNK_SIZE for fault tolerance.
const SNAPPY_BUFFER_SIZE: usize = 20 * 1024 * 1024;

// compresses the data into the buffer, resizing if necessary.
fn compression_helper(input: &[u8], output: &mut Vec<u8>) -> usize {
	let max_size = snappy::max_compressed_len(input.len());
	let buf_len = output.len();

	// resize if necessary, but in reality this will probably never happen.
	if max_size > buf_len {
		output.resize(max_size, 0);
	}

	match snappy::compress_into(&input, output) {
		Ok(size) => size,
		Err(snappy::Error::BufferTooSmall) => panic!("buffer too small although capacity ensured?"),
		Err(snappy::Error::InvalidInput) => panic!("invalid input error impossible in snappy_compress"),
	}
}

// shared portion of write_chunk
// returns either a (hash, compressed_size) pair or an io error.
fn write_chunk(raw_data: &[u8], compression_buffer: &mut Vec<u8>, path: &Path) -> Result<(H256, usize), Error> {
	let compressed_size = compression_helper(raw_data, compression_buffer);
	let compressed = &compression_buffer[..compressed_size];
	let hash = compressed.sha3();

	let mut file_path = path.to_owned();
	file_path.push(hash.hex());

	let mut file = try!(File::create(file_path));
	try!(file.write_all(compressed));

	Ok((hash, compressed_size))
}

/// Used to build block chunks.
struct BlockChunker<'a> {
	client: &'a BlockChainClient,
	// block, receipt rlp pairs.
	rlps: VecDeque<Bytes>,
	current_hash: H256,
	hashes: Vec<H256>,
	snappy_buffer: Vec<u8>,
}

impl<'a> BlockChunker<'a> {
	// Try to fill the buffers, moving backwards from current block hash.
	// Loops until we reach the genesis, and writes out the remainder.
	fn chunk_all(&mut self, genesis_hash: H256, path: &Path) -> Result<(), Error> {
		let mut loaded_size = 0;

		while self.current_hash != genesis_hash {
			let block = self.client.block(BlockID::Hash(self.current_hash)).unwrap();
			let view = BlockView::new(&block);
			let abridged_rlp = AbridgedBlock::from_block_view(&view).into_inner();

			let receipts = self.client.block_receipts(&self.current_hash).unwrap();

			let pair = {
				let mut pair_stream = RlpStream::new_list(2);
				pair_stream.append(&abridged_rlp).append(&receipts);
				pair_stream.out()
			};

			let new_loaded_size = loaded_size + pair.len();

			// cut off the chunk if too large
			if new_loaded_size > PREFERRED_CHUNK_SIZE {
				let header = view.header_view();
				try!(self.write_chunk(header.parent_hash(), header.number(), path));
				loaded_size = pair.len();
			} else {
				loaded_size = new_loaded_size;
			}

			self.rlps.push_front(pair);
			self.current_hash = view.header_view().parent_hash();
		}

		if loaded_size != 0 {
			// we don't store the genesis hash, so once we get to this point,
			// the "first" block will have number 1.
			try!(self.write_chunk(genesis_hash, 1, path));
		}

		Ok(())
	}

	// write out the data in the buffers to a chunk on disk
	fn write_chunk(&mut self, parent_hash: H256, number: u64, path: &Path) -> Result<(), Error> {
		trace!(target: "snapshot", "prepared block chunk with {} blocks", self.rlps.len());
		let mut rlp_stream = RlpStream::new_list(self.rlps.len() + 2);
		rlp_stream.append(&parent_hash).append(&number);
		for pair in self.rlps.drain(..) {
			rlp_stream.append(&pair);
		}

		let raw_data = rlp_stream.out();
		let (hash, size) = try!(write_chunk(&raw_data, &mut self.snappy_buffer, path));
		trace!(target: "snapshot", "wrote block chunk. hash: {}, size: {}, uncompressed size: {}", hash.hex(), size, raw_data.len());

		self.hashes.push(hash);
		Ok(())
	}
}

/// Create and write out all block chunks to disk, returning a vector of all
/// the hashes of block chunks created.
///
/// The path parameter is the directory to store the block chunks in.
/// This function assumes the directory exists already.
pub fn chunk_blocks(client: &BlockChainClient, best_block_hash: H256, genesis_hash: H256, path: &Path) -> Result<Vec<H256>, Error> {
	let mut chunker = BlockChunker {
		client: client,
		rlps: VecDeque::new(),
		current_hash: best_block_hash,
		hashes: Vec::new(),
		snappy_buffer: vec![0; SNAPPY_BUFFER_SIZE],
	};

	try!(chunker.chunk_all(genesis_hash, path));

	Ok(chunker.hashes)
}

/// State trie chunker.
struct StateChunker<'a> {
	hashes: Vec<H256>,
	rlps: Vec<Bytes>,
	cur_size: usize,
	snapshot_path: &'a Path,
	snappy_buffer: Vec<u8>,
}

impl<'a> StateChunker<'a> {
	// Push a key, value pair to be encoded.
	//
	// If the buffer is greater than the desired chunk size,
	// this will write out the data to disk.
	fn push(&mut self, key: Bytes, value: Bytes) -> Result<(), Error> {
		let pair = {
			let mut stream = RlpStream::new_list(2);
			stream.append(&key).append(&value);
			stream.out()
		};

		if self.cur_size + pair.len() >= PREFERRED_CHUNK_SIZE {
			try!(self.write_chunk());
		}

		self.cur_size += pair.len();
		self.rlps.push(pair);

		Ok(())
	}

	// Write out the buffer to disk, pushing the created chunk's hash to
	// the list.
	fn write_chunk(&mut self) -> Result<(), Error> {
		let mut stream = RlpStream::new();
		stream.append(&&self.rlps[..]);
		self.rlps.clear();

		let raw_data = stream.out();
		let (hash, compressed_size) = try!(write_chunk(&raw_data, &mut self.snappy_buffer, self.snapshot_path));
		trace!(target: "snapshot", "wrote state chunk. size: {}, uncompressed size: {}", compressed_size, raw_data.len());

		self.hashes.push(hash);
		self.cur_size = 0;

		Ok(())
	}
}

/// Walk the given state database starting from the given root,
/// creating chunks and writing them out.
///
/// Returns a list of hashes of chunks created, or any error it may
/// have encountered.
pub fn chunk_state(db: &HashDB, root: &H256, path: &Path) -> Result<Vec<H256>, Error> {
	let account_view = try!(TrieDB::new(db, &root));

	let mut chunker = StateChunker {
		hashes: Vec::new(),
		rlps: Vec::new(),
		cur_size: 0,
		snapshot_path: path,
		snappy_buffer: vec![0; SNAPPY_BUFFER_SIZE],
	};

	trace!(target: "snapshot", "beginning state chunking");

	// account_key here is the address' hash.
	for (account_key, account_data) in account_view.iter() {
		let account = AccountReader::from_thin_rlp(account_data);
		let account_key_hash = H256::from_slice(&account_key);

		let account_db = AccountDB::from_hash(db, account_key_hash);

		let fat_rlp = try!(account.to_fat_rlp(&account_db));
		try!(chunker.push(account_key, fat_rlp));
	}

	if chunker.cur_size != 0 {
		try!(chunker.write_chunk());
	}

	Ok(chunker.hashes)
}

// An alternate account structure, only used for reading the storage values
// out of the account as opposed to writing any.
struct AccountReader {
	nonce: U256,
	balance: U256,
	storage_root: H256,
	code_hash: H256,
}

impl AccountReader {
	// deserialize the account from rlp.
	fn from_thin_rlp(rlp: &[u8]) -> Self {
		let r: Rlp = Rlp::new(rlp);

		AccountReader {
			nonce: r.val_at(0),
			balance: r.val_at(1),
			storage_root: r.val_at(2),
			code_hash: r.val_at(3),
		}
	}

	// walk the account's storage trie, returning an RLP item containing the
	// account properties and the storage.
	fn to_fat_rlp(&self, hash_db: &HashDB) -> Result<Bytes, Error> {
		let db = try!(TrieDB::new(hash_db, &self.storage_root));

		let mut pairs = Vec::new();

		for (k, v) in db.iter() {
			pairs.push((k, v));
		}

		let mut stream = RlpStream::new_list(pairs.len());

		for (k, v) in pairs {
			stream.begin_list(2).append(&k).append(&v);
		}

		let pairs_rlp = stream.out();

		let mut account_stream = RlpStream::new_list(5);
		account_stream.append(&self.nonce)
					  .append(&self.balance)
					  .append(&self.storage_root);

		account_stream.begin_list(2);
		if self.code_hash == SHA3_NULL_RLP {
			account_stream.append(&true).append(&hash_db.get(&self.code_hash).unwrap());
		} else {
			account_stream.append(&false).append_empty_data();
		}

		account_stream.append(&pairs_rlp);

		Ok(account_stream.out())
	}
}

/// Manifest data.
pub struct ManifestData {
	/// List of state chunk hashes.
	pub state_hashes: Vec<H256>,
	/// List of block chunk hashes.
	pub block_hashes: Vec<H256>,
	/// The final, expected state root.
	pub state_root: H256,
}

impl ManifestData {
	/// Encode the manifest data to.
	pub fn to_rlp(self) -> Bytes {
		let mut stream = RlpStream::new_list(3);
		stream.append(&self.state_hashes);
		stream.append(&self.block_hashes);
		stream.append(&self.state_root);

		stream.out()
	}

	/// Try to restore manifest data from raw bytes interpreted as RLP.
	pub fn from_rlp(raw: &[u8]) -> Result<Self, DecoderError> {
		let decoder = UntrustedRlp::new(raw);

		let state_hashes: Vec<H256> = try!(try!(decoder.at(0)).as_val());
		let block_hashes: Vec<H256> = try!(try!(decoder.at(1)).as_val());
		let state_root: H256 = try!(try!(decoder.at(2)).as_val());

		Ok(ManifestData {
			state_hashes: state_hashes,
			block_hashes: block_hashes,
			state_root: state_root,
		})
	}
}