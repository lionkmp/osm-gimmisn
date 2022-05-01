/*
 * Copyright 2021 Miklos Vajna. All rights reserved.
 * Use of this source code is governed by a BSD-style license that can be
 * found in the LICENSE file.
 */

#![deny(warnings)]
#![warn(clippy::all)]
#![warn(missing_docs)]

//! Abstractions to help writing unit tests: filesystem, network, etc.

use anyhow::Context as _;
use std::cell::RefCell;
use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

/// File system interface.
pub trait FileSystem {
    /// Test whether a path exists.
    fn path_exists(&self, path: &str) -> bool;

    /// Return the last modification time of a file.
    fn getmtime(&self, path: &str) -> anyhow::Result<f64>;

    /// Opens a file for reading in binary mode.
    fn open_read(&self, path: &str) -> anyhow::Result<Rc<RefCell<dyn Read>>>;

    /// Opens a file for writing in binary mode.
    fn open_write(&self, path: &str) -> anyhow::Result<Rc<RefCell<dyn Write>>>;

    /// Removes a file.
    fn unlink(&self, path: &str) -> anyhow::Result<()>;

    /// Super-mkdir; create a leaf directory and all intermediate ones.
    fn makedirs(&self, path: &str) -> anyhow::Result<()>;

    /// Return a list containing the names of the files in the directory.
    fn listdir(&self, path: &str) -> anyhow::Result<Vec<String>>;

    /// Read the entire contents of a file into a string.
    fn read_to_string(&self, path: &str) -> anyhow::Result<String> {
        let stream = self.open_read(path)?;
        let mut guard = stream.borrow_mut();
        let mut bytes: Vec<u8> = Vec::new();
        guard.read_to_end(&mut bytes).unwrap();
        Ok(String::from_utf8(bytes)?)
    }

    /// Write the entire string to a file.
    fn write_from_string(&self, string: &str, path: &str) -> anyhow::Result<()> {
        let stream = self.open_write(path)?;
        let mut guard = stream.borrow_mut();
        Ok(guard.write_all(string.as_bytes())?)
    }
}

pub use system::StdFileSystem;

/// Network interface.
pub trait Network {
    /// Opens an URL. Empty data means HTTP GET, otherwise it means a HTTP POST.
    fn urlopen(&self, url: &str, data: &str) -> anyhow::Result<String>;
}

pub use system::StdNetwork;

/// Time interface.
pub trait Time {
    /// Calculates the current Unix timestamp from GMT.
    fn now(&self) -> i64;

    /// Delay execution for a given number of seconds.
    fn sleep(&self, seconds: u64);

    /// Allows accessing the implementing struct.
    fn as_any(&self) -> &dyn std::any::Any;
}

pub use system::StdTime;

/// Subprocess interface.
pub trait Subprocess {
    /// Runs a commmand, capturing its output.
    fn run(&self, args: Vec<String>) -> anyhow::Result<String>;

    /// Terminates the current process with the specified exit code.
    fn exit(&self, code: i32);

    /// Allows accessing the implementing struct.
    fn as_any(&self) -> &dyn std::any::Any;
}

pub use system::StdSubprocess;

/// Unit testing interface.
pub trait Unit {
    /// Injects a fake error.
    fn make_error(&self) -> anyhow::Result<()>;
}

pub use system::StdUnit;

/// Configuration file reader.
#[derive(Clone)]
pub struct Ini {
    config: configparser::ini::Ini,
    root: String,
}

impl Ini {
    fn new(
        file_system: &Arc<dyn FileSystem>,
        config_path: &str,
        root: &str,
    ) -> anyhow::Result<Self> {
        let mut config = configparser::ini::Ini::new();
        if let Err(err) = config.read(file_system.read_to_string(config_path)?) {
            return Err(anyhow::anyhow!("failed to load {}: {}", config_path, err));
        }
        Ok(Ini {
            config,
            root: String::from(root),
        })
    }

    /// Gets the directory which is writable.
    pub fn get_workdir(&self) -> String {
        format!("{}/workdir", self.root)
    }

    /// Gets the abs paths of ref housenumbers.
    pub fn get_reference_housenumber_paths(&self) -> anyhow::Result<Vec<String>> {
        let value = self
            .config
            .get("wsgi", "reference_housenumbers")
            .context("no wsgi.reference_housenumbers in config")?;
        let relpaths = value.split(' ');
        Ok(relpaths
            .map(|relpath| format!("{}/{}", self.root, relpath))
            .collect())
    }

    /// Gets the abs path of ref streets.
    pub fn get_reference_street_path(&self) -> anyhow::Result<String> {
        let relpath = self
            .config
            .get("wsgi", "reference_street")
            .context("no wsgi.reference_street in config")?;
        Ok(format!("{}/{}", self.root, relpath))
    }

    /// Gets the abs path of ref citycounts.
    pub fn get_reference_citycounts_path(&self) -> anyhow::Result<String> {
        let relpath = self
            .config
            .get("wsgi", "reference_citycounts")
            .context("no wsgi.reference_citycounts in config")?;
        Ok(format!("{}/{}", self.root, relpath))
    }

    /// Gets the abs path of ref zipcounts.
    pub fn get_reference_zipcounts_path(&self) -> anyhow::Result<String> {
        let relpath = self
            .config
            .get("wsgi", "reference_zipcounts")
            .context("no wsgi.reference_zipcounts in config")?;
        Ok(format!("{}/{}", self.root, relpath))
    }

    /// Gets the global URI prefix.
    pub fn get_uri_prefix(&self) -> anyhow::Result<String> {
        self.config
            .get("wsgi", "uri_prefix")
            .context("no wsgi.uri_prefix in config")
    }

    fn get_with_fallback(&self, key: &str, fallback: &str) -> String {
        match self.config.get("wsgi", key) {
            Some(value) => value,
            None => String::from(fallback),
        }
    }

    /// Gets the TCP port to be used.
    pub fn get_tcp_port(&self) -> anyhow::Result<i64> {
        Ok(self.get_with_fallback("tcp_port", "8000").parse::<i64>()?)
    }

    /// Gets the URI of the overpass instance to be used.
    pub fn get_overpass_uri(&self) -> String {
        self.get_with_fallback("overpass_uri", "https://overpass-api.de")
    }

    /// Should the cron job update inactive relations?
    pub fn get_cron_update_inactive(&self) -> bool {
        let value = self.get_with_fallback("cron_update_inactive", "False");
        value == "True"
    }
}

/// Context owns global state which is set up once and then read everywhere.
#[derive(Clone)]
pub struct Context {
    root: String,
    ini: Ini,
    network: Arc<dyn Network>,
    time: Arc<dyn Time>,
    subprocess: Arc<dyn Subprocess>,
    unit: Arc<dyn Unit>,
    file_system: Arc<dyn FileSystem>,
}

impl Context {
    /// Creates a new Context.
    pub fn new(prefix: &str) -> anyhow::Result<Self> {
        let root = format!("{}/{}", env!("CARGO_MANIFEST_DIR"), prefix);
        let network = Arc::new(StdNetwork {});
        let time = Arc::new(StdTime {});
        let subprocess = Arc::new(StdSubprocess {});
        let unit = Arc::new(StdUnit {});
        let file_system: Arc<dyn FileSystem> = Arc::new(StdFileSystem {});
        let ini = Ini::new(&file_system, &format!("{}/wsgi.ini", root), &root)?;
        Ok(Context {
            root,
            ini,
            network,
            time,
            subprocess,
            unit,
            file_system,
        })
    }

    /// Make a path absolute, taking the repo root as a base dir.
    pub fn get_abspath(&self, rel_path: &str) -> String {
        format!("{}/{}", self.root, rel_path)
    }

    /// Gets the ini file.
    pub fn get_ini(&self) -> &Ini {
        &self.ini
    }

    /// Gets the network implementation.
    pub fn get_network(&self) -> &Arc<dyn Network> {
        &self.network
    }

    /// Sets the network implementation.
    pub fn set_network(&mut self, network: &Arc<dyn Network>) {
        self.network = network.clone();
    }

    /// Gets the time implementation.
    pub fn get_time(&self) -> &Arc<dyn Time> {
        &self.time
    }

    /// Sets the time implementation.
    pub fn set_time(&mut self, time: &Arc<dyn Time>) {
        self.time = time.clone();
    }

    /// Gets the subprocess implementation.
    pub fn get_subprocess(&self) -> &Arc<dyn Subprocess> {
        &self.subprocess
    }

    /// Sets the subprocess implementation.
    pub fn set_subprocess(&mut self, subprocess: &Arc<dyn Subprocess>) {
        self.subprocess = subprocess.clone();
    }

    /// Gets the testing interface.
    pub fn get_unit(&self) -> &Arc<dyn Unit> {
        &self.unit
    }

    /// Sets the unit implementation.
    pub fn set_unit(&mut self, unit: &Arc<dyn Unit>) {
        self.unit = unit.clone();
    }

    /// Gets the file system implementation.
    pub fn get_file_system(&self) -> &Arc<dyn FileSystem> {
        &self.file_system
    }

    /// Sets the file system implementation.
    pub fn set_file_system(&mut self, file_system: &Arc<dyn FileSystem>) {
        self.file_system = file_system.clone();
    }
}

pub mod system;
#[cfg(test)]
pub mod tests;
