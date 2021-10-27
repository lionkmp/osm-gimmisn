/*
 * Copyright 2021 Miklos Vajna. All rights reserved.
 * Use of this source code is governed by a BSD-style license that can be
 * found in the LICENSE file.
 */

#![deny(warnings)]
#![warn(clippy::all)]
#![warn(missing_docs)]

//! The cron module allows doing nightly tasks.

use crate::areas;
use crate::cache;
use crate::context;
use crate::i18n;
use crate::overpass_query;
use crate::stats;
use crate::util;
use anyhow::Context;
use chrono::Datelike;
use pyo3::prelude::*;
use std::cmp::Reverse;
use std::collections::HashMap;
use std::collections::HashSet;
use std::io::BufRead;
use std::io::Write;
use std::ops::DerefMut;

/// Sets up logging.
pub fn setup_logging(ctx: &context::Context) -> anyhow::Result<()> {
    let config = simplelog::ConfigBuilder::new()
        .set_time_format("%Y-%m-%d %H:%M:%S".into())
        .set_time_to_local(true)
        .build();
    let logpath = ctx.get_abspath("workdir/cron.log")?;
    let file = std::fs::File::create(logpath)?;
    simplelog::CombinedLogger::init(vec![
        simplelog::TermLogger::new(
            simplelog::LevelFilter::Info,
            config.clone(),
            simplelog::TerminalMode::Stdout,
            simplelog::ColorChoice::Never,
        ),
        simplelog::WriteLogger::new(simplelog::LevelFilter::Info, config, file),
    ])?;

    Ok(())
}

#[pyfunction]
fn py_setup_logging(ctx: context::PyContext) -> PyResult<()> {
    match setup_logging(&ctx.context).context("setup_logging() failed") {
        Ok(value) => Ok(value),
        Err(err) => Err(pyo3::exceptions::PyOSError::new_err(format!("{:?}", err))),
    }
}

/// Sleeps to respect overpass rate limit.
fn overpass_sleep(ctx: &context::Context) {
    loop {
        let sleep = overpass_query::overpass_query_need_sleep(ctx);
        if sleep == 0 {
            break;
        }
        log::info!("overpass_sleep: waiting for {} seconds", sleep);
        ctx.get_time().sleep(sleep as u64);
    }
}

/// Decides if we should retry a query or not.
fn should_retry(retry: i32) -> bool {
    retry < 20
}

/// Update the OSM street list of all relations.
fn update_osm_streets(
    ctx: &context::Context,
    relations: &mut areas::Relations,
    update: bool,
) -> anyhow::Result<()> {
    for relation_name in relations.get_active_names()? {
        let relation = relations.get_relation(&relation_name)?;
        if !update && std::path::Path::new(&relation.get_files().get_osm_streets_path()?).exists() {
            continue;
        }
        log::info!("update_osm_streets: start: {}", relation_name);
        let mut retry = 0;
        while should_retry(retry) {
            if retry > 0 {
                log::info!("update_osm_streets: try #{}", retry);
            }
            retry += 1;
            overpass_sleep(ctx);
            let query = relation.get_osm_streets_query()?;
            let buf = match overpass_query::overpass_query(ctx, query) {
                Ok(value) => value,
                Err(err) => {
                    log::info!("update_osm_streets: http error: {:?}", err);
                    continue;
                }
            };
            if relation.get_files().write_osm_streets(ctx, &buf)? == 0 {
                log::info!("update_osm_streets: short write");
                continue;
            }
            break;
        }
        log::info!("update_osm_streets: end: {}", relation_name);
    }

    Ok(())
}

#[pyfunction]
fn py_update_osm_streets(
    ctx: context::PyContext,
    mut relations: areas::PyRelations,
    update: bool,
) -> PyResult<()> {
    match update_osm_streets(&ctx.context, &mut relations.relations, update)
        .context("update_osm_streets() failed")
    {
        Ok(value) => Ok(value),
        Err(err) => Err(pyo3::exceptions::PyOSError::new_err(format!("{:?}", err))),
    }
}

/// Update the OSM housenumber list of all relations.
fn update_osm_housenumbers(
    ctx: &context::Context,
    relations: &mut areas::Relations,
    update: bool,
) -> anyhow::Result<()> {
    for relation_name in relations.get_active_names()? {
        let relation = relations.get_relation(&relation_name)?;
        if !update
            && std::path::Path::new(&relation.get_files().get_osm_housenumbers_path()?).exists()
        {
            continue;
        }
        log::info!("update_osm_housenumbers: start: {}", relation_name);
        let mut retry = 0;
        while should_retry(retry) {
            if retry > 0 {
                log::info!("update_osm_housenumbers: try #{}", retry);
            }
            retry += 1;
            overpass_sleep(ctx);
            let query = relation.get_osm_housenumbers_query()?;
            let buf = match overpass_query::overpass_query(ctx, query) {
                Ok(value) => value,
                Err(err) => {
                    log::info!("update_osm_housenumbers: http error: {:?}", err);
                    continue;
                }
            };
            if relation.get_files().write_osm_housenumbers(ctx, &buf)? == 0 {
                log::info!("update_osm_housenumbers: short write");
                continue;
            }
            break;
        }
        log::info!("update_osm_housenumbers: end: {}", relation_name);
    }

    Ok(())
}

#[pyfunction]
fn py_update_osm_housenumbers(
    ctx: context::PyContext,
    mut relations: areas::PyRelations,
    update: bool,
) -> PyResult<()> {
    match update_osm_housenumbers(&ctx.context, &mut relations.relations, update)
        .context("update_osm_housenumbers() failed")
    {
        Ok(value) => Ok(value),
        Err(err) => Err(pyo3::exceptions::PyOSError::new_err(format!("{:?}", err))),
    }
}

/// Update the reference housenumber list of all relations.
fn update_ref_housenumbers(
    ctx: &context::Context,
    relations: &mut areas::Relations,
    update: bool,
) -> anyhow::Result<()> {
    for relation_name in relations.get_active_names()? {
        let relation = relations.get_relation(&relation_name)?;
        if !update
            && std::path::Path::new(&relation.get_files().get_ref_housenumbers_path()?).exists()
        {
            continue;
        }
        let references = ctx.get_ini().get_reference_housenumber_paths()?;
        let streets = relation.get_config().should_check_missing_streets();
        if streets == "only" {
            continue;
        }

        log::info!("update_ref_housenumbers: start: {}", relation_name);
        if let Err(err) = relation.write_ref_housenumbers(&references) {
            log::info!("update_osm_housenumbers: failed: {:?}", err);
            continue;
        }
        log::info!("update_ref_housenumbers: end: {}", relation_name);
    }

    Ok(())
}

/// Update the reference street list of all relations.
fn update_ref_streets(
    ctx: &context::Context,
    relations: &mut areas::Relations,
    update: bool,
) -> anyhow::Result<()> {
    for relation_name in relations.get_active_names()? {
        let relation = relations.get_relation(&relation_name)?;
        if !update && std::path::Path::new(&relation.get_files().get_ref_streets_path()?).exists() {
            continue;
        }
        let reference = ctx.get_ini().get_reference_street_path()?;
        let streets = relation.get_config().should_check_missing_streets();
        if streets == "no" {
            continue;
        }

        log::info!("update_ref_streets: start: {}", relation_name);
        relation.write_ref_streets(&reference)?;
        log::info!("update_ref_streets: end: {}", relation_name);
    }

    Ok(())
}

/// Update the relation's house number coverage stats.
fn update_missing_housenumbers(
    ctx: &context::Context,
    relations: &mut areas::Relations,
    update: bool,
) -> anyhow::Result<()> {
    log::info!("update_missing_housenumbers: start");
    for relation_name in relations.get_active_names()? {
        let mut relation = relations.get_relation(&relation_name)?;
        if !update
            && std::path::Path::new(&relation.get_files().get_housenumbers_percent_path()?).exists()
        {
            continue;
        }
        let streets = relation.get_config().should_check_missing_streets();
        if streets == "only" {
            continue;
        }

        let orig_language = i18n::get_language();
        relation.write_missing_housenumbers()?;
        for language in ["en", "hu"] {
            i18n::set_language(language)?;
            cache::get_missing_housenumbers_html(ctx, &mut relation)?;
        }
        i18n::set_language(&orig_language)?;
        cache::get_missing_housenumbers_txt(ctx, &mut relation)?;
    }
    log::info!("update_missing_housenumbers: end");

    Ok(())
}

/// Update the relation's street coverage stats.
fn update_missing_streets(relations: &mut areas::Relations, update: bool) -> anyhow::Result<()> {
    log::info!("update_missing_streets: start");
    for relation_name in relations.get_active_names()? {
        let relation = relations.get_relation(&relation_name)?;
        if !update
            && std::path::Path::new(&relation.get_files().get_streets_percent_path()?).exists()
        {
            continue;
        }
        let streets = relation.get_config().should_check_missing_streets();
        if streets == "no" {
            continue;
        }

        relation.write_missing_streets()?;
    }
    log::info!("update_missing_streets: end");

    Ok(())
}

/// Update the relation's "additional streets" stats.
fn update_additional_streets(relations: &mut areas::Relations, update: bool) -> anyhow::Result<()> {
    log::info!("update_additional_streets: start");
    for relation_name in relations.get_active_names()? {
        let relation = relations.get_relation(&relation_name)?;
        if !update
            && std::path::Path::new(&relation.get_files().get_streets_additional_count_path()?)
                .exists()
        {
            continue;
        }
        let streets = relation.get_config().should_check_missing_streets();
        if streets == "no" {
            continue;
        }

        relation.write_additional_streets()?;
    }
    log::info!("update_additional_streets: end");

    Ok(())
}

/// Writes a daily .count file.
fn write_count_path(
    ctx: &context::Context,
    count_path: &str,
    house_numbers: &HashSet<String>,
) -> anyhow::Result<()> {
    let stream = ctx.get_file_system().open_write(count_path)?;
    let mut guard = stream.lock().unwrap();
    let house_numbers_len = house_numbers.len().to_string();
    Ok(guard.write_all(house_numbers_len.as_bytes())?)
}

/// Writes a daily .citycount file.
fn write_city_count_path(
    ctx: &context::Context,
    city_count_path: &str,
    cities: &HashMap<String, HashSet<String>>,
) -> anyhow::Result<()> {
    let stream = ctx.get_file_system().open_write(city_count_path)?;
    let mut guard = stream.lock().unwrap();
    let mut cities: Vec<_> = cities.iter().map(|(key, value)| (key, value)).collect();
    cities.sort_by_key(|(key, _value)| util::get_sort_key(key).unwrap());
    cities.dedup();
    // Locale-aware sort, by key.
    for (key, value) in cities {
        let line = format!("{}\t{}\n", key, value.len());
        guard.write_all(line.as_bytes())?;
    }

    Ok(())
}

/// Counts the # of all house numbers as of today.
fn update_stats_count(ctx: &context::Context, today: &str) -> anyhow::Result<()> {
    let statedir = ctx.get_abspath("workdir/stats")?;
    let csv_path = format!("{}/{}.csv", statedir, today);
    if !ctx.get_file_system().path_exists(&csv_path) {
        return Ok(());
    }
    let count_path = format!("{}/{}.count", statedir, today);
    let city_count_path = format!("{}/{}.citycount", statedir, today);
    let mut house_numbers: HashSet<String> = HashSet::new();
    let mut cities: HashMap<String, HashSet<String>> = HashMap::new();
    let mut first = true;
    let valid_settlements = util::get_valid_settlements(ctx)?;
    let stream = ctx.get_file_system().open_read(&csv_path)?;
    let mut guard = stream.lock().unwrap();
    let reader = std::io::BufReader::new(guard.deref_mut());
    for line in reader.lines() {
        let line = line?.to_string();
        if line.starts_with("<?xml") {
            // Not a CSV, reject.
            break;
        }
        if first {
            // Ignore the oneliner header.
            first = false;
            continue;
        }
        // postcode, city name, street name, house number, user
        let cells: Vec<String> = line.split('\t').map(|i| i.into()).collect();
        // Ignore last column, which is the user who touched the object last.
        house_numbers.insert(cells[0..4].join("\t"));
        let city_key = util::get_city_key(&cells[0], &cells[1], &valid_settlements)?;
        let city_value = cells[2..4].join("\t");
        let entry = cities.entry(city_key).or_insert_with(HashSet::new);
        entry.insert(city_value);
    }
    write_count_path(ctx, &count_path, &house_numbers)?;
    write_city_count_path(ctx, &city_count_path, &cities)
}

#[pyfunction]
fn py_update_stats_count(ctx: context::PyContext, today: &str) -> PyResult<()> {
    match update_stats_count(&ctx.context, today).context("update_stats_count() failed") {
        Ok(value) => Ok(value),
        Err(err) => Err(pyo3::exceptions::PyOSError::new_err(format!("{:?}", err))),
    }
}

/// Counts the top housenumber editors as of today.
fn update_stats_topusers(ctx: &context::Context, today: &str) -> anyhow::Result<()> {
    let statedir = ctx.get_abspath("workdir/stats")?;
    let csv_path = format!("{}/{}.csv", statedir, today);
    if !ctx.get_file_system().path_exists(&csv_path) {
        return Ok(());
    }
    let topusers_path = format!("{}/{}.topusers", statedir, today);
    let usercount_path = format!("{}/{}.usercount", statedir, today);
    let mut users: HashMap<String, u64> = HashMap::new();
    {
        let stream = ctx.get_file_system().open_read(&csv_path)?;
        let mut guard = stream.lock().unwrap();
        let reader = std::io::BufReader::new(guard.deref_mut());
        for line in reader.lines() {
            let line = line?.to_string();
            let cells: Vec<String> = line.split('\t').map(|i| i.into()).collect();
            // Only care about the last column.
            let user = cells[cells.len() - 1].clone();
            let entry = users.entry(user).or_insert(0);
            (*entry) += 1;
        }
    }
    {
        let stream = ctx.get_file_system().open_write(&topusers_path)?;
        let mut guard = stream.lock().unwrap();
        let mut users: Vec<_> = users.iter().map(|(key, value)| (key, value)).collect();
        users.sort_by_key(|i| Reverse(i.1));
        users.dedup();
        users = users[0..std::cmp::min(20, users.len())].to_vec();
        for user in users {
            let line = format!("{} {}\n", user.1, user.0);
            guard.write_all(line.as_bytes())?;
        }
    }

    let stream = ctx.get_file_system().open_write(&usercount_path)?;
    let mut guard = stream.lock().unwrap();
    let line = format!("{}\n", users.len());
    Ok(guard.write_all(line.as_bytes())?)
}

#[pyfunction]
fn py_update_stats_topusers(ctx: context::PyContext, today: &str) -> PyResult<()> {
    match update_stats_topusers(&ctx.context, today).context("update_stats_topusers() failed") {
        Ok(value) => Ok(value),
        Err(err) => Err(pyo3::exceptions::PyOSError::new_err(format!("{:?}", err))),
    }
}

/// Performs the update of workdir/stats/ref.count.
fn update_stats_refcount(ctx: &context::Context, state_dir: &str) -> anyhow::Result<()> {
    let mut count = 0;
    {
        let stream = ctx
            .get_file_system()
            .open_read(&ctx.get_ini().get_reference_citycounts_path()?)?;
        let mut guard = stream.lock().unwrap();
        let mut read = guard.deref_mut();
        let mut csv_read = util::CsvRead::new(&mut read);
        let mut first = true;
        for result in csv_read.records() {
            let row = result?;
            if first {
                first = false;
                continue;
            }

            count += row[1].parse::<i32>()?;
        }
    }

    let stream = ctx
        .get_file_system()
        .open_write(&format!("{}/ref.count", state_dir))?;
    let mut guard = stream.lock().unwrap();
    Ok(guard.write_all(format!("{}\n", count).as_bytes())?)
}

/// Performs the update of country-level stats.
fn update_stats(ctx: &context::Context, overpass: bool) -> anyhow::Result<()> {
    // Fetch house numbers for the whole country.
    log::info!("update_stats: start, updating whole-country csv");
    let query = String::from_utf8(util::get_content(
        &ctx.get_abspath("data/street-housenumbers-hungary.txt")?,
    )?)?;
    let statedir = ctx.get_abspath("workdir/stats")?;
    std::fs::create_dir_all(&statedir)?;
    let now = chrono::NaiveDateTime::from_timestamp(ctx.get_time().now(), 0);
    let today = now.format("%Y-%m-%d").to_string();
    let csv_path = format!("{}/{}.csv", statedir, today);

    if overpass {
        log::info!("update_stats: talking to overpass");
        let mut retry = 0;
        while should_retry(retry) {
            if retry > 0 {
                log::info!("update_stats: try #{}", retry);
            }
            retry += 1;
            overpass_sleep(ctx);
            let response = match overpass_query::overpass_query(ctx, query.clone()) {
                Ok(value) => value,
                Err(err) => {
                    log::info!("update_stats: http error: {}", err);
                    continue;
                }
            };
            let stream = ctx.get_file_system().open_write(&csv_path)?;
            let mut guard = stream.lock().unwrap();
            guard.write_all(response.as_bytes())?;
            break;
        }
    }

    update_stats_count(ctx, &today)?;
    update_stats_topusers(ctx, &today)?;
    update_stats_refcount(ctx, &statedir)?;

    // Remove old CSV files as they are created daily and each is around 11M.
    for entry in std::fs::read_dir(&statedir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().unwrap() != "csv" {
            continue;
        }

        let metadata = std::fs::metadata(&path)?;
        let last_modified = metadata.modified()?.elapsed()?.as_secs();

        if last_modified >= 24 * 3600 * 7 && metadata.is_file() {
            std::fs::remove_file(&path)?;
            let file_name = path.file_name().unwrap().to_str().unwrap();
            log::info!("update_stats: removed old {}", file_name);
        }
    }

    log::info!("update_stats: generating json");
    let json_path = format!("{}/stats.json", &statedir);
    stats::generate_json(ctx, &statedir, &json_path)?;

    log::info!("update_stats: end");

    Ok(())
}

#[pyfunction]
fn py_update_stats(ctx: context::PyContext, overpass: bool) -> PyResult<()> {
    match update_stats(&ctx.context, overpass).context("update_stats() failed") {
        Ok(value) => Ok(value),
        Err(err) => Err(pyo3::exceptions::PyOSError::new_err(format!("{:?}", err))),
    }
}

/// Performs the actual nightly task.
fn our_main(
    ctx: &context::Context,
    relations: &mut areas::Relations,
    mode: &str,
    update: bool,
    overpass: bool,
) -> anyhow::Result<()> {
    if mode == "all" || mode == "stats" {
        update_stats(ctx, overpass)?;
    }
    if mode == "all" || mode == "relations" {
        update_osm_streets(ctx, relations, update)?;
        update_osm_housenumbers(ctx, relations, update)?;
        update_ref_streets(ctx, relations, update)?;
        update_ref_housenumbers(ctx, relations, update)?;
        update_missing_streets(relations, update)?;
        update_missing_housenumbers(ctx, relations, update)?;
        update_additional_streets(relations, update)?;
    }

    let pid = std::process::id();
    let stream = std::fs::File::open(format!("/proc/{}/status", pid))?;
    let reader = std::io::BufReader::new(stream);
    for line in reader.lines() {
        let line = line?.to_string();
        if line.starts_with("VmPeak:") {
            let vm_peak = line.trim();
            log::info!("our_main: {}", vm_peak);
            break;
        }
    }
    let err = ctx.get_unit().make_error();
    if !err.is_empty() {
        return Err(anyhow::anyhow!(err));
    }

    Ok(())
}

#[pyfunction]
fn py_our_main(
    ctx: context::PyContext,
    mut relations: areas::PyRelations,
    mode: &str,
    update: bool,
    overpass: bool,
) -> PyResult<()> {
    match our_main(
        &ctx.context,
        &mut relations.relations,
        mode,
        update,
        overpass,
    )
    .context("our_main() failed")
    {
        Ok(value) => Ok(value),
        Err(err) => Err(pyo3::exceptions::PyOSError::new_err(format!("{:?}", err))),
    }
}

/// Commandline interface to this module.
pub fn main(
    argv: &[String],
    _stream: &mut dyn Write,
    ctx: &context::Context,
) -> anyhow::Result<()> {
    let mut relations = areas::Relations::new(ctx)?;

    let args = clap::App::new("osm-gimmisn")
        .arg(
            clap::Arg::with_name("refcounty")
                .long("refcounty")
                .takes_value(true)
                .help("limit the list of relations to a given refcounty"),
        )
        .arg(
            clap::Arg::with_name("refsettlement")
                .long("refsettlement")
                .takes_value(true)
                .help("limit the list of relations to a given refsettlement"),
        )
        .arg(
            clap::Arg::with_name("no-update") // default: true
                .long("no-update")
                .help("don't update existing state of relations"),
        )
        .arg(
            clap::Arg::with_name("mode")
                .long("mode")
                .takes_value(true)
                .default_value("relations")
                .help("only perform the given sub-task or all of them [all, stats or relations]"),
        )
        .arg(
            clap::Arg::with_name("no-overpass") // default: true
                .long("no-overpass")
                .help("when updating stats, don't perform any overpass update"),
        )
        .get_matches_from_safe(argv)?;

    let start = ctx.get_time().now();
    // Query inactive relations once a month.
    let now = chrono::NaiveDateTime::from_timestamp(start, 0);
    let first_day_of_month = now.date().day() == 1;
    relations.activate_all(ctx.get_ini().get_cron_update_inactive() || first_day_of_month);
    let refcounty = args.value_of("refcounty").map(|value| value.to_string());
    relations.limit_to_refcounty(&refcounty)?;
    // Use map(), which handles optional values.
    let refsettlement = args
        .value_of("refsettlement")
        .map(|value| value.to_string());
    relations.limit_to_refsettlement(&refsettlement)?;
    let update = !args.is_present("no-update");
    let overpass = !args.is_present("no-overpass");
    match our_main(
        ctx,
        &mut relations,
        args.value_of("mode").unwrap(),
        update,
        overpass,
    ) {
        Ok(_) => (),
        Err(err) => log::error!("main: unhandled error: {:?}", err),
    }
    let duration = chrono::Duration::seconds(ctx.get_time().now() - start);
    let seconds = duration.num_seconds() % 60;
    let minutes = duration.num_minutes() % 60;
    let hours = duration.num_hours();
    log::info!(
        "main: finished in {}:{:0>2}:{:0>2}",
        hours,
        minutes,
        seconds
    );

    Ok(())
}

#[pyfunction]
fn py_cron_main(argv: Vec<String>, stdout: PyObject, ctx: &context::PyContext) -> PyResult<()> {
    let mut stream = context::PyAnyWrite { write: stdout };
    match main(&argv, &mut stream, &ctx.context) {
        Ok(value) => Ok(value),
        Err(err) => Err(pyo3::exceptions::PyOSError::new_err(format!(
            "main() failed: {}",
            err.to_string()
        ))),
    }
}

/// Registers Python wrappers of Rust structs into the Python module.
pub fn register_python_symbols(module: &PyModule) -> PyResult<()> {
    module.add_function(pyo3::wrap_pyfunction!(py_setup_logging, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(py_update_osm_streets, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(py_update_osm_housenumbers, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(py_update_stats_count, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(py_update_stats_topusers, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(py_update_stats, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(py_our_main, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(py_cron_main, module)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use context::FileSystem;
    use std::sync::Arc;

    /// Tests overpass_sleep(): the case when no sleep is needed.
    #[test]
    fn test_overpass_sleep_no_sleep() {
        let mut ctx = context::tests::make_test_context().unwrap();
        let routes = vec![context::tests::URLRoute::new(
            /*url=*/ "https://overpass-api.de/api/status",
            /*data_path=*/ "",
            /*result_path=*/ "tests/network/overpass-status-happy.txt",
        )];
        let network = context::tests::TestNetwork::new(&routes);
        let network_arc: Arc<dyn context::Network> = Arc::new(network);
        ctx.set_network(&network_arc);
        let time = context::tests::make_test_time();
        let time_arc: Arc<dyn context::Time> = Arc::new(time);
        ctx.set_time(&time_arc);

        overpass_sleep(&ctx);

        let time = time_arc
            .as_any()
            .downcast_ref::<context::tests::TestTime>()
            .unwrap();
        assert_eq!(time.get_sleep(), 0);
    }

    /// Tests overpass_sleep(): the case when sleep is needed.
    #[test]
    fn test_overpass_sleep_need_sleep() {
        let mut ctx = context::tests::make_test_context().unwrap();
        let routes = vec![
            context::tests::URLRoute::new(
                /*url=*/ "https://overpass-api.de/api/status",
                /*data_path=*/ "",
                /*result_path=*/ "tests/network/overpass-status-wait.txt",
            ),
            context::tests::URLRoute::new(
                /*url=*/ "https://overpass-api.de/api/status",
                /*data_path=*/ "",
                /*result_path=*/ "tests/network/overpass-status-happy.txt",
            ),
        ];
        let network = context::tests::TestNetwork::new(&routes);
        let network_arc: Arc<dyn context::Network> = Arc::new(network);
        ctx.set_network(&network_arc);
        let time = context::tests::make_test_time();
        let time_arc: Arc<dyn context::Time> = Arc::new(time);
        ctx.set_time(&time_arc);

        overpass_sleep(&ctx);

        let time = time_arc
            .as_any()
            .downcast_ref::<context::tests::TestTime>()
            .unwrap();
        assert_eq!(time.get_sleep(), 12);
    }

    /// Tests update_ref_housenumbers().
    #[test]
    fn test_update_ref_housenumbers() {
        let ctx = context::tests::make_test_context().unwrap();
        let mut relations = areas::Relations::new(&ctx).unwrap();
        for relation_name in relations.get_active_names().unwrap() {
            if relation_name != "gazdagret" && relation_name != "ujbuda" {
                let mut relation = relations.get_relation(&relation_name).unwrap();
                let mut config = relation.get_config().clone();
                config.set_active(false);
                relation.set_config(&config);
                relations.set_relation(&relation_name, &relation);
            }
        }
        let path = ctx
            .get_abspath("workdir/street-housenumbers-reference-gazdagret.lst")
            .unwrap();
        let expected = String::from_utf8(util::get_content(&path).unwrap()).unwrap();
        std::fs::remove_file(&path).unwrap();

        update_ref_housenumbers(&ctx, &mut relations, /*update=*/ true).unwrap();

        let expected_mtime: std::time::Duration;
        {
            let metadata = std::fs::metadata(&path).unwrap();
            let modified = metadata.modified().unwrap();
            expected_mtime = modified
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap();
        }

        update_ref_housenumbers(&ctx, &mut relations, /*update=*/ false).unwrap();

        let actual_mtime: std::time::Duration;
        {
            let metadata = std::fs::metadata(&path).unwrap();
            let modified = metadata.modified().unwrap();
            actual_mtime = modified
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap();
        }
        assert_eq!(actual_mtime, expected_mtime);
        let actual = String::from_utf8(util::get_content(&path).unwrap()).unwrap();
        assert_eq!(actual, expected);
        // Make sure housenumber ref is not created for the streets=only case.
        let ujbuda_path = ctx
            .get_abspath("workdir/street-housenumbers-reference-ujbuda.lst")
            .unwrap();
        assert_eq!(std::path::Path::new(&ujbuda_path).exists(), false);
    }

    /// Tests update_ref_streets().
    #[test]
    fn test_update_ref_streets() {
        let ctx = context::tests::make_test_context().unwrap();
        let mut relations = areas::Relations::new(&ctx).unwrap();
        for relation_name in relations.get_active_names().unwrap() {
            // gellerthegy is streets=no
            if relation_name != "gazdagret" && relation_name != "gellerthegy" {
                let mut relation = relations.get_relation(&relation_name).unwrap();
                let mut config = relation.get_config().clone();
                config.set_active(false);
                relation.set_config(&config);
                relations.set_relation(&relation_name, &relation);
            }
        }
        let path = ctx
            .get_abspath("workdir/streets-reference-gazdagret.lst")
            .unwrap();
        let expected = String::from_utf8(util::get_content(&path).unwrap()).unwrap();
        std::fs::remove_file(&path).unwrap();

        update_ref_streets(&ctx, &mut relations, /*update=*/ true).unwrap();

        let expected_mtime: std::time::Duration;
        {
            let metadata = std::fs::metadata(&path).unwrap();
            let modified = metadata.modified().unwrap();
            expected_mtime = modified
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap();
        }

        update_ref_streets(&ctx, &mut relations, /*update=*/ false).unwrap();

        let actual_mtime: std::time::Duration;
        {
            let metadata = std::fs::metadata(&path).unwrap();
            let modified = metadata.modified().unwrap();
            actual_mtime = modified
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap();
        }
        assert_eq!(actual_mtime, expected_mtime);
        let actual = String::from_utf8(util::get_content(&path).unwrap()).unwrap();
        assert_eq!(actual, expected);
        // Make sure street ref is not created for the streets=no case.
        let ujbuda_path = ctx
            .get_abspath("workdir/street-reference-ujbuda.lst")
            .unwrap();
        assert_eq!(std::path::Path::new(&ujbuda_path).exists(), false);
    }

    /// Tests update_missing_housenumbers().
    #[test]
    fn test_update_missing_housenumbers() {
        let ctx = context::tests::make_test_context().unwrap();
        let mut relations = areas::Relations::new(&ctx).unwrap();
        for relation_name in relations.get_active_names().unwrap() {
            // ujbuda is streets=only
            if relation_name != "gazdagret" && relation_name != "ujbuda" {
                let mut relation = relations.get_relation(&relation_name).unwrap();
                let mut config = relation.get_config().clone();
                config.set_active(false);
                relation.set_config(&config);
                relations.set_relation(&relation_name, &relation);
            }
        }
        let path = ctx.get_abspath("workdir/gazdagret.percent").unwrap();
        let expected = String::from_utf8(util::get_content(&path).unwrap()).unwrap();
        std::fs::remove_file(&path).unwrap();

        update_missing_housenumbers(&ctx, &mut relations, /*update=*/ true).unwrap();

        let expected_mtime: std::time::Duration;
        {
            let metadata = std::fs::metadata(&path).unwrap();
            let modified = metadata.modified().unwrap();
            expected_mtime = modified
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap();
        }

        update_missing_housenumbers(&ctx, &mut relations, /*update=*/ false).unwrap();

        let actual_mtime: std::time::Duration;
        {
            let metadata = std::fs::metadata(&path).unwrap();
            let modified = metadata.modified().unwrap();
            actual_mtime = modified
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap();
        }
        assert_eq!(actual_mtime, expected_mtime);
        let actual = String::from_utf8(util::get_content(&path).unwrap()).unwrap();
        assert_eq!(actual, expected);
        // Make sure housenumber stat is not created for the streets=only case.
        let ujbuda_path = format!("workdir/ujbuda.percent");
        assert_eq!(std::path::Path::new(&ujbuda_path).exists(), false);
    }

    /// Tests update_missing_streets().
    #[test]
    fn test_update_missing_streets() {
        let ctx = context::tests::make_test_context().unwrap();
        let file_system = context::tests::TestFileSystem::new();
        let mut relations = areas::Relations::new(&ctx).unwrap();
        for relation_name in relations.get_active_names().unwrap() {
            // gellerthegy is streets=no
            if relation_name != "gazdagret" && relation_name != "gellerthegy" {
                let mut relation = relations.get_relation(&relation_name).unwrap();
                let mut config = relation.get_config().clone();
                config.set_active(false);
                relation.set_config(&config);
                relations.set_relation(&relation_name, &relation);
            }
        }
        let path = ctx
            .get_abspath("workdir/gazdagret-streets.percent")
            .unwrap();
        let expected = String::from_utf8(util::get_content(&path).unwrap()).unwrap();
        std::fs::remove_file(&path).unwrap();
        update_missing_streets(&mut relations, /*update=*/ true).unwrap();
        let mtime = file_system.getmtime(&path).unwrap();

        update_missing_streets(&mut relations, /*update=*/ false).unwrap();

        assert_eq!(file_system.getmtime(&path).unwrap(), mtime);
        let actual = String::from_utf8(util::get_content(&path).unwrap()).unwrap();
        assert_eq!(actual, expected);
        // Make sure street stat is not created for the streets=no case.
        assert_eq!(
            file_system.path_exists(
                &ctx.get_abspath("workdir/gellerthegy-streets.percent")
                    .unwrap()
            ),
            false
        );
    }

    /// Tests update_additional_streets().
    #[test]
    fn test_update_additional_streets() {
        let ctx = context::tests::make_test_context().unwrap();
        let file_system = context::tests::TestFileSystem::new();
        let mut relations = areas::Relations::new(&ctx).unwrap();
        for relation_name in relations.get_active_names().unwrap() {
            // gellerthegy is streets=no
            if relation_name != "gazdagret" && relation_name != "gellerthegy" {
                let mut relation = relations.get_relation(&relation_name).unwrap();
                let mut config = relation.get_config().clone();
                config.set_active(false);
                relation.set_config(&config);
                relations.set_relation(&relation_name, &relation);
            }
        }
        let path = ctx
            .get_abspath("workdir/gazdagret-additional-streets.count")
            .unwrap();
        let expected: String = "1".into();
        if file_system.path_exists(&path) {
            std::fs::remove_file(&path).unwrap();
        }
        update_additional_streets(&mut relations, /*update=*/ true).unwrap();
        let mtime = file_system.getmtime(&path).unwrap();

        update_additional_streets(&mut relations, /*update=*/ false).unwrap();

        assert_eq!(file_system.getmtime(&path).unwrap(), mtime);
        let actual = String::from_utf8(util::get_content(&path).unwrap()).unwrap();
        assert_eq!(actual, expected);
        // Make sure street stat is not created for the streets=no case.
        assert_eq!(
            file_system.path_exists(
                &ctx.get_abspath("workdir/gellerthegy-additional-streets.count")
                    .unwrap()
            ),
            false
        );
    }

    /// Tests update_osm_housenumbers().
    #[test]
    fn test_update_osm_housenumbers() {
        let mut ctx = context::tests::make_test_context().unwrap();
        let routes = vec![
            context::tests::URLRoute::new(
                /*url=*/ "https://overpass-api.de/api/status",
                /*data_path=*/ "",
                /*result_path=*/ "tests/network/overpass-status-happy.txt",
            ),
            context::tests::URLRoute::new(
                /*url=*/ "https://overpass-api.de/api/interpreter",
                /*data_path=*/ "",
                /*result_path=*/ "tests/network/overpass-housenumbers-gazdagret.csv",
            ),
        ];
        let network = context::tests::TestNetwork::new(&routes);
        let network_arc: Arc<dyn context::Network> = Arc::new(network);
        ctx.set_network(&network_arc);
        let mut relations = areas::Relations::new(&ctx).unwrap();
        for relation_name in relations.get_active_names().unwrap() {
            if relation_name != "gazdagret" {
                let mut relation = relations.get_relation(&relation_name).unwrap();
                let mut config = relation.get_config().clone();
                config.set_active(false);
                relation.set_config(&config);
                relations.set_relation(&relation_name, &relation);
            }
        }
        let path = ctx
            .get_abspath("workdir/street-housenumbers-gazdagret.csv")
            .unwrap();
        let expected = String::from_utf8(util::get_content(&path).unwrap()).unwrap();
        std::fs::remove_file(&path).unwrap();
        update_osm_housenumbers(&ctx, &mut relations, /*update=*/ true).unwrap();
        let mtime = ctx.get_file_system().getmtime(&path).unwrap();

        update_osm_housenumbers(&ctx, &mut relations, /*update=*/ false).unwrap();

        assert_eq!(ctx.get_file_system().getmtime(&path).unwrap(), mtime);
        let actual = String::from_utf8(util::get_content(&path).unwrap()).unwrap();
        assert_eq!(actual, expected);
    }
}
