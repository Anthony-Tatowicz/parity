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

#![cfg_attr(feature="nightly", feature(custom_derive, plugin))]
#![cfg_attr(feature="nightly", plugin(serde_macros))]

extern crate libc;
extern crate rand;
extern crate serde;
extern crate serde_json;
extern crate rustc_serialize;
extern crate crypto as rcrypto;
extern crate tiny_keccak;
// reexport it nicely
extern crate ethkey as _ethkey;

pub mod dir;
pub mod ethkey;

mod account;
mod json;
mod crypto;

mod error;
mod ethstore;
mod import;
mod presale;
mod random;
mod secret_store;

pub use self::account::SafeAccount;
pub use self::error::Error;
pub use self::ethstore::EthStore;
pub use self::import::import_accounts;
pub use self::presale::PresaleWallet;
pub use self::secret_store::SecretStore;

