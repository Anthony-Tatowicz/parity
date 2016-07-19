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

//! Ethcore client application.

#![warn(missing_docs)]
#![cfg_attr(feature="dev", feature(plugin))]
#![cfg_attr(feature="dev", plugin(clippy))]
#![cfg_attr(feature="dev", allow(useless_format))]
#![cfg_attr(feature="dev", allow(match_bool))]

extern crate docopt;
extern crate num_cpus;
extern crate rustc_serialize;
extern crate ethcore_util as util;
extern crate ethcore;
extern crate ethsync;
#[macro_use]
extern crate log as rlog;
extern crate env_logger;
extern crate ctrlc;
extern crate fdlimit;
extern crate time;
extern crate number_prefix;
extern crate rpassword;
extern crate semver;
extern crate ethcore_ipc as ipc;
extern crate ethcore_ipc_nano as nanoipc;
#[macro_use]
extern crate hyper; // for price_info.rs
extern crate json_ipc_server as jsonipc;

extern crate ethcore_ipc_hypervisor as hypervisor;
extern crate ethcore_rpc;

extern crate ethcore_signer;
extern crate ansi_term;
#[macro_use]
extern crate lazy_static;
extern crate regex;
extern crate isatty;

#[cfg(feature = "dapps")]
extern crate ethcore_dapps;

mod commands;
mod cache;
mod upgrade;
mod setup_log;
mod rpc;
mod dapps;
mod informant;
mod io_handler;
mod cli;
mod configuration;
mod migration;
mod signer;
mod rpc_apis;
mod url;
mod helpers;
mod params;
mod deprecated;
mod dir;
mod modules;

use std::sync::{Arc, Mutex, Condvar};
use std::path::Path;
use std::{env, process};
use ctrlc::CtrlC;
use fdlimit::raise_fd_limit;
use util::network_settings::NetworkSettings;
use util::{Colour, version, H256, NetworkConfiguration, U256};
use util::journaldb::Algorithm;
use util::panics::{MayPanic, ForwardPanic, PanicHandler};
use ethcore::client::{Mode, Switch, DatabaseCompactionProfile, VMType};
use ethcore::service::ClientService;
use ethcore::account_provider::AccountProvider;
use ethcore::miner::{Miner, MinerService, ExternalMiner, MinerOptions};
use ethsync::SyncConfig;
use migration::migrate;
use informant::Informant;

use rpc::{HttpServer, IpcServer, HttpConfiguration, IpcConfiguration};
use signer::SignerServer;
use dapps::WebappServer;
use io_handler::ClientIoHandler;
use configuration::{Configuration, IOPasswordReader};
use params::{SpecType, Pruning, AccountsConfig, GasPricerConfig, MinerExtras};
use helpers::to_client_config;
use dir::Directories;
use setup_log::{LoggerConfig, setup_log};
use cache::CacheConfig;

fn main() {
	let conf = Configuration::parse(env::args()).unwrap_or_else(|e| e.exit());
	match new_execute(conf) {
		Ok(result) => {
			print!("{}", result);
		},
		Err(err) => {
			print!("{}", err);
			process::exit(1);
		}
	}
}

fn new_execute(conf: Configuration) -> Result<String, String> {
	let cmd = try!(conf.into_command(&IOPasswordReader));
	commands::execute(cmd)
}

#[derive(Debug, PartialEq)]
pub struct RunCmd {
	cache_config: CacheConfig,
	directories: Directories,
	spec: SpecType,
	pruning: Pruning,
	/// Some if execution should be daemonized. Contains pid_file path.
	daemon: Option<String>,
	logger_config: LoggerConfig,
	miner_options: MinerOptions,
	http_conf: HttpConfiguration,
	ipc_conf: IpcConfiguration,
	net_conf: NetworkConfiguration,
	network_id: Option<U256>,
	acc_conf: AccountsConfig,
	gas_pricer: GasPricerConfig,
	miner_extras: MinerExtras,
	mode: Mode,
	tracing: Switch,
	compaction: DatabaseCompactionProfile,
	vm_type: VMType,
	enable_network: bool,
	geth_compatibility: bool,
	signer_port: Option<u16>,
	net_settings: NetworkSettings,
	dapps_conf: dapps::Configuration,
	signer_conf: signer::Configuration,
	ui: bool,
}

fn execute(cmd: RunCmd) -> Result<(), String> {
	// increase max number of open files
	raise_fd_limit();

	// set up logger
	let logger = try!(setup_log(&cmd.logger_config));

	// set up panic handler
	let panic_handler = PanicHandler::new_in_arc();

	// create directories used by parity
	try!(cmd.directories.create_dirs());

	// load spec
	let spec = try!(cmd.spec.spec());

	// load genesis hash
	let genesis_hash = spec.genesis_header().hash();

	// select pruning algorithm
	let algorithm = cmd.pruning.to_algorithm(&cmd.directories, genesis_hash);

	// prepare client_path
	let client_path = cmd.directories.client_path(genesis_hash, algorithm);

	// execute upgrades
	try!(execute_upgrades(&cmd.directories, genesis_hash, algorithm));

	// run in daemon mode
	if let Some(pid_file) = cmd.daemon {
		try!(daemonize(pid_file));
	}

	// display info about used pruning algorithm
	info!("Starting {}", Colour::White.bold().paint(version()));
	info!("Using state DB journalling strategy {}", Colour::White.bold().paint(algorithm.as_str()));

	// display warning about using experimental journaldb alorithm
	if !algorithm.is_stable() {
		warn!("Your chosen strategy is {}! You can re-run with --pruning to change.", Colour::Red.bold().paint("unstable"));
	}

	// create sync config
	let mut sync_config = SyncConfig::default();
	sync_config.network_id = match cmd.network_id {
		Some(id) => id,
		None => spec.network_id(),
	};

	// prepare account provider
	let account_provider = Arc::new(try!(prepare_account_provider(&cmd.directories, cmd.acc_conf)));

	// create miner
	let miner = Miner::new(cmd.miner_options, cmd.gas_pricer.into(), spec, Some(account_provider.clone()));
	miner.set_author(cmd.miner_extras.author);
	miner.set_gas_floor_target(cmd.miner_extras.gas_floor_target);
	miner.set_gas_ceil_target(cmd.miner_extras.gas_ceil_target);
	miner.set_extra_data(cmd.miner_extras.extra_data);
	miner.set_transactions_limit(cmd.miner_extras.transactions_limit);

	// create client config
	let client_config = to_client_config(
		&cmd.cache_config,
		&cmd.directories,
		genesis_hash,
		cmd.mode,
		cmd.tracing,
		cmd.pruning,
		cmd.compaction,
		cmd.vm_type
	);

	// load spec
	// TODO: make it clonable and load it only once!
	let spec = try!(cmd.spec.spec());

	// set up bootnodes
	let mut net_conf = cmd.net_conf;
	// TODO: this should happen only if bootnodes where not specified in commandline
	net_conf.boot_nodes = spec.nodes.clone();

	// create client
	let service = try!(ClientService::start(
		client_config,
		spec,
		Path::new(&client_path),
		miner.clone(),
	).map_err(|e| format!("Client service error: {:?}", e)));

	// forward panics from service
	panic_handler.forward_from(&service);

	// take handle to client
	let client = service.client();

	// create external miner
	let external_miner = Arc::new(ExternalMiner::default());

	// create sync object
	let (sync_provider, manage_network, chain_notify) = try!(modules::sync(
		sync_config, net_conf.into(), client.clone()
	).map_err(|e| format!("Sync error: {}", e)));

	service.set_notify(&chain_notify);

	// start network
	if cmd.enable_network {
		chain_notify.start();
	}

	// set up dependencies for rpc servers
	let deps_for_rpc_apis = Arc::new(rpc_apis::Dependencies {
		signer_port: cmd.signer_port,
		signer_queue: Arc::new(rpc_apis::ConfirmationsQueue::default()),
		client: client.clone(),
		sync: sync_provider.clone(),
		net: manage_network.clone(),
		secret_store: account_provider.clone(),
		miner: miner.clone(),
		external_miner: external_miner.clone(),
		logger: logger.clone(),
		settings: Arc::new(cmd.net_settings.clone()),
		allow_pending_receipt_query: !cmd.geth_compatibility,
		net_service: manage_network.clone()
	});

	let dependencies = rpc::Dependencies {
		panic_handler: panic_handler.clone(),
		apis: deps_for_rpc_apis.clone(),
	};

	// start rpc servers
	let http_server = try!(rpc::new_http(cmd.http_conf, &dependencies));
	let ipc_server = try!(rpc::new_ipc(cmd.ipc_conf, &dependencies));

	let dapps_deps = dapps::Dependencies {
		panic_handler: panic_handler.clone(),
		apis: deps_for_rpc_apis.clone(),
	};

	// start dapps server
	let dapps_server = try!(dapps::new(cmd.dapps_conf.clone(), dapps_deps));

	let signer_deps = signer::Dependencies {
		panic_handler: panic_handler.clone(),
		apis: deps_for_rpc_apis.clone(),
	};

	// start signer server
	let signer_server = try!(signer::start(cmd.signer_conf, signer_deps));

	let io_handler = Arc::new(ClientIoHandler {
		client: service.client(),
		info: Informant::new(cmd.logger_config.color),
		sync: sync_provider.clone(),
		net: manage_network.clone(),
		accounts: account_provider.clone(),
	});
	service.register_io_handler(io_handler).expect("Error registering IO handler");

	// start ui
	if cmd.ui {
		if !cmd.dapps_conf.enabled {
			return Err("Cannot use UI command with Dapps turned off.".into())
		}
		url::open(&format!("http://{}:{}/", cmd.dapps_conf.interface, cmd.dapps_conf.port));
	}

	// Handle exit
	wait_for_exit(panic_handler, http_server, ipc_server, dapps_server, signer_server);

	Ok(())
}

#[cfg(not(windows))]
fn daemonize(pid_file: String) -> Result<(), String> {
	extern crate daemonize;

	daemonize::Daemonize::new()
			.pid_file(pid_file)
			.chown_pid_file(true)
			.start()
			.map(|_| ())
			.map_err(|e| format!("Couldn't daemonize; {}", e))
}

#[cfg(windows)]
fn daemonize(_conf: &Configuration) -> ! {
}

fn execute_upgrades(dirs: &Directories, genesis_hash: H256, pruning: Algorithm) -> Result<(), String> {
	match upgrade::upgrade(Some(&dirs.db)) {
		Ok(upgrades_applied) if upgrades_applied > 0 => {
			debug!("Executed {} upgrade scripts - ok", upgrades_applied);
		},
		Err(e) => {
			return Err(format!("Error upgrading parity data: {:?}", e));
		},
		_ => {},
	}

	let client_path = dirs.client_path(genesis_hash, pruning);
	migrate(&client_path, pruning).map_err(|e| format!("{}", e))
}

fn prepare_account_provider(dirs: &Directories, cfg: AccountsConfig) -> Result<AccountProvider, String> {
	use ethcore::ethstore::{import_accounts, EthStore};
	use ethcore::ethstore::dir::{GethDirectory, DirectoryType, DiskDirectory};

	// TODO: read passwords from files
	let passwords = Vec::<String>::new();

	if cfg.import_keys {
		let t = if cfg.testnet {
			DirectoryType::Testnet
		} else {
			DirectoryType::Main
		};

		let from = GethDirectory::open(t);
		let to = DiskDirectory::create(dirs.keys.clone()).unwrap();
		// ignore error, cause geth may not exist
		let _ = import_accounts(&from, &to);
	}

	let dir = Box::new(DiskDirectory::create(dirs.keys.clone()).unwrap());
	let account_service = AccountProvider::new(Box::new(EthStore::open_with_iterations(dir, cfg.iterations).unwrap()));

	for a in cfg.unlocked_accounts {
		if passwords.iter().find(|p| account_service.unlock_account_permanently(a, (*p).clone()).is_ok()).is_none() {
			return Err(format!("No password given to unlock account {}. Pass the password using `--password`.", a));
		}
	}

	Ok(account_service)
}

fn wait_for_exit(
	panic_handler: Arc<PanicHandler>,
	_http_server: Option<HttpServer>,
	_ipc_server: Option<IpcServer>,
	_dapps_server: Option<WebappServer>,
	_signer_server: Option<SignerServer>
	) {
	let exit = Arc::new(Condvar::new());

	// Handle possible exits
	let e = exit.clone();
	CtrlC::set_handler(move || { e.notify_all(); });

	// Handle panics
	let e = exit.clone();
	panic_handler.on_panic(move |_reason| { e.notify_all(); });

	// Wait for signal
	let mutex = Mutex::new(());
	let _ = exit.wait(mutex.lock().unwrap());
	info!("Finishing work, please wait...");
}
