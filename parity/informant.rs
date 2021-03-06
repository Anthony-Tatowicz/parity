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

extern crate ansi_term;
use self::ansi_term::Colour::{White, Yellow, Green, Cyan, Blue};
use self::ansi_term::Style;

use std::sync::{Arc};
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::time::{Instant, Duration};
use std::ops::{Deref, DerefMut};
use isatty::{stdout_isatty};
use ethsync::{SyncProvider, ManageNetwork};
use util::{Uint, RwLock, Mutex, H256, Colour};
use ethcore::client::*;
use ethcore::views::BlockView;
use number_prefix::{binary_prefix, Standalone, Prefixed};

pub struct Informant {
	chain_info: RwLock<Option<BlockChainInfo>>,
	cache_info: RwLock<Option<BlockChainCacheSize>>,
	report: RwLock<Option<ClientReport>>,
	last_tick: RwLock<Instant>,
	with_color: bool,
	client: Arc<Client>,
	sync: Option<Arc<SyncProvider>>,
	net: Option<Arc<ManageNetwork>>,
	last_import: Mutex<Instant>,
	skipped: AtomicUsize,
}

trait MillisecondDuration {
	fn as_milliseconds(&self) -> u64;
}

impl MillisecondDuration for Duration {
	fn as_milliseconds(&self) -> u64 {
		self.as_secs() * 1000 + self.subsec_nanos() as u64 / 1000000
	}
}

impl Informant {
	/// Make a new instance potentially `with_color` output.
	pub fn new(client: Arc<Client>, sync: Option<Arc<SyncProvider>>, net: Option<Arc<ManageNetwork>>, with_color: bool) -> Self {
		Informant {
			chain_info: RwLock::new(None),
			cache_info: RwLock::new(None),
			report: RwLock::new(None),
			last_tick: RwLock::new(Instant::now()),
			with_color: with_color,
			client: client,
			sync: sync,
			net: net,
			last_import: Mutex::new(Instant::now()),
			skipped: AtomicUsize::new(0),
		}
	}

	fn format_bytes(b: usize) -> String {
		match binary_prefix(b as f64) {
			Standalone(bytes)   => format!("{} bytes", bytes),
			Prefixed(prefix, n) => format!("{:.0} {}B", n, prefix),
		}
	}


	#[cfg_attr(feature="dev", allow(match_bool))]
	pub fn tick(&self) {
		let elapsed = self.last_tick.read().elapsed();
		if elapsed < Duration::from_secs(5) {
			return;
		}

		let chain_info = self.client.chain_info();
		let queue_info = self.client.queue_info();
		let cache_info = self.client.blockchain_cache_info();
		let network_config = self.net.as_ref().map(|n| n.network_config());
		let sync_status = self.sync.as_ref().map(|s| s.status());

		let importing = queue_info.unverified_queue_size + queue_info.verified_queue_size > 3
			|| self.sync.as_ref().map_or(false, |s| s.status().is_major_syncing());
		if !importing && elapsed < Duration::from_secs(30) {
			return;
		}

		*self.last_tick.write() = Instant::now();

		let mut write_report = self.report.write();
		let report = self.client.report();

		let paint = |c: Style, t: String| match self.with_color && stdout_isatty() {
			true => format!("{}", c.paint(t)),
			false => t,
		};

		info!(target: "import", "{}   {}   {}",
			match importing {
				true => format!("{} {}   {}   {}+{} Qed", 
					paint(White.bold(), format!("{:>8}", format!("#{}", chain_info.best_block_number))),
					paint(White.bold(), format!("{}", chain_info.best_block_hash)),
					{
						let last_report = match write_report.deref() { &Some(ref last_report) => last_report.clone(), _ => ClientReport::default() };
						format!("{} blk/s {} tx/s {} Mgas/s",  
							paint(Yellow.bold(), format!("{:4}", ((report.blocks_imported - last_report.blocks_imported) * 1000) as u64 / elapsed.as_milliseconds())),
							paint(Yellow.bold(), format!("{:4}", ((report.transactions_applied - last_report.transactions_applied) * 1000) as u64 / elapsed.as_milliseconds())),
							paint(Yellow.bold(), format!("{:3}", ((report.gas_processed - last_report.gas_processed) / From::from(elapsed.as_milliseconds() * 1000)).low_u64()))
						)
					},
					paint(Green.bold(), format!("{:5}", queue_info.unverified_queue_size)),
					paint(Green.bold(), format!("{:5}", queue_info.verified_queue_size))
				),
				false => String::new(),
			},
			match (&sync_status, &network_config) {
				(&Some(ref sync_info), &Some(ref net_config)) => format!("{}{}/{}/{} peers",
					match importing {
						true => format!("{}   ", paint(Green.bold(), format!("{:>8}", format!("#{}", sync_info.last_imported_block_number.unwrap_or(chain_info.best_block_number))))),
						false => String::new(),
					},
					paint(Cyan.bold(), format!("{:2}", sync_info.num_active_peers)),
					paint(Cyan.bold(), format!("{:2}", sync_info.num_peers)),
					paint(Cyan.bold(), format!("{:2}", net_config.ideal_peers))
				),
				_ => String::new(),
			},
			format!("{} db {} chain {} queue{}",
				paint(Blue.bold(), format!("{:>8}", Informant::format_bytes(report.state_db_mem))),
				paint(Blue.bold(), format!("{:>8}", Informant::format_bytes(cache_info.total()))),
				paint(Blue.bold(), format!("{:>8}", Informant::format_bytes(queue_info.mem_used))),
				match sync_status {
					Some(ref sync_info) => format!(" {} sync", paint(Blue.bold(), format!("{:>8}", Informant::format_bytes(sync_info.mem_used)))),
					_ => String::new(),
				}
			)
		);

		*self.chain_info.write().deref_mut() = Some(chain_info);
		*self.cache_info.write().deref_mut() = Some(cache_info);
		*write_report.deref_mut() = Some(report);
	}
}

impl ChainNotify for Informant {
	fn new_blocks(&self, _imported: Vec<H256>, _invalid: Vec<H256>, enacted: Vec<H256>, _retracted: Vec<H256>, _sealed: Vec<H256>, duration: u64) {
		let mut last_import = self.last_import.lock();
		if Instant::now() > *last_import + Duration::from_secs(1) {
			let queue_info = self.client.queue_info();
			let importing = queue_info.unverified_queue_size + queue_info.verified_queue_size > 3
				|| self.sync.as_ref().map_or(false, |s| s.status().is_major_syncing());
			if !importing {
				if let Some(block) = enacted.last().and_then(|h| self.client.block(BlockID::Hash(h.clone()))) {
					let view = BlockView::new(&block);
					let header = view.header();
					let tx_count = view.transactions_count();
					let size = block.len();
					let skipped = self.skipped.load(AtomicOrdering::Relaxed);
					info!(target: "import", "Imported {} {} ({} txs, {} Mgas, {} ms, {} KiB){}",
					Colour::White.bold().paint(format!("#{}", header.number())),
					Colour::White.bold().paint(format!("{}", header.hash())),
					Colour::Yellow.bold().paint(format!("{}", tx_count)),
					Colour::Yellow.bold().paint(format!("{:.2}", header.gas_used.low_u64() as f32 / 1000000f32)),
					Colour::Purple.bold().paint(format!("{:.2}", duration as f32 / 1000000f32)),
					Colour::Blue.bold().paint(format!("{:.2}", size as f32 / 1024f32)),
					if skipped > 0 { format!(" + another {} block(s)", Colour::Red.bold().paint(format!("{}", skipped))) } else { String::new() }
					);
					*last_import = Instant::now();
				}
			}
			self.skipped.store(0, AtomicOrdering::Relaxed);
		} else {
			self.skipped.fetch_add(enacted.len(), AtomicOrdering::Relaxed);
		}
	}
}

