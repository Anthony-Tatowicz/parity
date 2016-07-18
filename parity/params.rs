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

use std::str::FromStr;
use ethcore::spec::Spec;
use ethcore::ethereum;
use util::{contents, DatabaseConfig, journaldb, H256};
use util::journaldb::Algorithm;
use dir::Directories;

#[derive(Debug, PartialEq)]
pub enum SpecType {
	Mainnet,
	Testnet,
	Olympic,
	Custom(String),
}

impl Default for SpecType {
	fn default() -> Self {
		SpecType::Mainnet
	}
}

impl FromStr for SpecType {
	type Err = String;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let spec = match s {
			"frontier" | "homestead" | "mainnet" => SpecType::Mainnet,
			"morden" | "testnet" => SpecType::Testnet,
			"olympic" => SpecType::Olympic,
			other => SpecType::Custom(other.into()),
		};
		Ok(spec)
	}
}

impl SpecType {
	pub fn spec(&self) -> Result<Spec, String> {
		match *self {
			SpecType::Mainnet => Ok(ethereum::new_frontier()),
			SpecType::Testnet => Ok(ethereum::new_morden()),
			SpecType::Olympic => Ok(ethereum::new_olympic()),
			SpecType::Custom(ref file) => Ok(Spec::load(&try!(contents(file).map_err(|_| "Could not load specification file."))))
		}
	}
}

#[derive(Debug, PartialEq)]
pub enum Pruning {
	Specific(Algorithm),
	Auto,
}

impl Default for Pruning {
	fn default() -> Self {
		Pruning::Auto
	}
}

impl FromStr for Pruning {
	type Err = String;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s {
			"auto" => Ok(Pruning::Auto),
			other => other.parse().map(Pruning::Specific),
		}
	}
}

impl Pruning {
	pub fn to_algorithm(&self, dirs: &Directories, genesis_hash: H256) -> Algorithm {
		match *self {
			Pruning::Specific(algo) => algo,
			Pruning::Auto => Self::find_best_db(dirs, genesis_hash),
		}
	}

	fn find_best_db(dirs: &Directories, genesis_hash: H256) -> Algorithm {
		let mut algo_types = Algorithm::all_types();

		// if all dbs have the same latest era, the last element is the default one
		algo_types.push(Algorithm::default());

		algo_types.into_iter().max_by_key(|i| {
			let mut client_path = dirs.client_path(genesis_hash, *i);
			client_path.push("state");
			let db = journaldb::new(client_path.to_str().unwrap(), *i, DatabaseConfig::default());
			trace!(target: "parity", "Looking for best DB: {} at {:?}", i, db.latest_era());
			db.latest_era()
		}).unwrap()
	}
}

#[derive(Debug, PartialEq)]
pub struct ResealPolicy {
	pub own: bool,
	pub external: bool,
}

impl Default for ResealPolicy {
	fn default() -> Self {
		ResealPolicy {
			own: true,
			external: true,
		}
	}
}

impl FromStr for ResealPolicy {
	type Err = String;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let (own, external) = match s {
			"none" => (false, false),
			"own" => (true, false),
			"ext" => (false, true),
			"all" => (true, true),
			x => return Err(format!("Invalid reseal value: {}", x)),
		};

		let reseal = ResealPolicy {
			own: own,
			external: external,
		};

		Ok(reseal)
	}
}


#[cfg(test)]
mod tests {
	use util::journaldb::Algorithm;
	use super::{SpecType, Pruning, ResealPolicy};

	#[test]
	fn test_spec_type_parsing() {
		assert_eq!(SpecType::Mainnet, "frontier".parse().unwrap());
		assert_eq!(SpecType::Mainnet, "homestead".parse().unwrap());
		assert_eq!(SpecType::Mainnet, "mainnet".parse().unwrap());
		assert_eq!(SpecType::Testnet, "testnet".parse().unwrap());
		assert_eq!(SpecType::Testnet, "morden".parse().unwrap());
		assert_eq!(SpecType::Olympic, "olympic".parse().unwrap());
	}

	#[test]
	fn test_spec_type_default() {
		assert_eq!(SpecType::Mainnet, SpecType::default());
	}

	#[test]
	fn test_pruning_parsing() {
		assert_eq!(Pruning::Auto, "auto".parse().unwrap());
		assert_eq!(Pruning::Specific(Algorithm::Archive), "archive".parse().unwrap());
		assert_eq!(Pruning::Specific(Algorithm::EarlyMerge), "earlymerge".parse().unwrap());
		assert_eq!(Pruning::Specific(Algorithm::OverlayRecent), "overlayrecent".parse().unwrap());
		assert_eq!(Pruning::Specific(Algorithm::RefCounted), "refcounted".parse().unwrap());
	}

	#[test]
	fn test_pruning_default() {
		assert_eq!(Pruning::Auto, Pruning::default());
	}

	#[test]
	fn test_reseal_policy_parsing() {
		let none = ResealPolicy { own: false, external: false };
		let own = ResealPolicy { own: true, external: false };
		let ext = ResealPolicy { own: false, external: true };
		let all = ResealPolicy { own: true, external: true };
		assert_eq!(none, "none".parse().unwrap());
		assert_eq!(own, "own".parse().unwrap());
		assert_eq!(ext, "ext".parse().unwrap());
		assert_eq!(all, "all".parse().unwrap());
	}

	#[test]
	fn test_reseal_policy_default() {
		let all = ResealPolicy { own: true, external: true };
		assert_eq!(all, ResealPolicy::default());
	}
}
