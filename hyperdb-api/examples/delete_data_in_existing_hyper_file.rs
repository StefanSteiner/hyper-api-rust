// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Example: Delete data from an existing Hyper file
//!
//! This example demonstrates how to:
//! - Connect to an existing Hyper file
//! - Delete data using SQL DELETE statements
//! - Use subqueries in DELETE statements
//!
//! This is a Rust port of the C++ example `delete_data_in_existing_hyper_file.cpp`.
//!
//! Run with: cargo run -p hyperdb-api --example `delete_data_in_existing_hyper_file`

use hyperdb_api::{
    escape_name, escape_string_literal, Catalog, Connection, CreateMode, Date, HyperProcess,
    Inserter, Result, SqlType, TableDefinition,
};

/// Creates the Orders table definition.
fn orders_table() -> TableDefinition {
    TableDefinition::new("Orders")
        .add_required_column("Address ID", SqlType::small_int())
        .add_required_column("Customer ID", SqlType::text())
        .add_required_column("Order Date", SqlType::date())
        .add_required_column("Order ID", SqlType::text())
        .add_nullable_column("Ship Date", SqlType::date())
        .add_nullable_column("Ship Mode", SqlType::text())
}

/// Creates the Customer table definition.
fn customer_table() -> TableDefinition {
    TableDefinition::new("Customer")
        .add_required_column("Customer ID", SqlType::text())
        .add_required_column("Customer Name", SqlType::text())
        .add_required_column("Loyalty Reward Points", SqlType::big_int())
        .add_required_column("Segment", SqlType::text())
}

/// Creates a sample Hyper file with Customer and Orders data.
fn create_sample_data(hyper: &HyperProcess, path: &str) -> Result<()> {
    // Create the data directory if it doesn't exist
    if let Some(parent) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(parent)?;
    }

    let connection = Connection::new(hyper, path, CreateMode::CreateAndReplace)?;
    let catalog = Catalog::new(&connection);

    // Create tables
    let orders_def = orders_table();
    let customer_def = customer_table();
    catalog.create_table(&orders_def)?;
    catalog.create_table(&customer_def)?;

    // Insert Customer data
    {
        let mut inserter = Inserter::new(&connection, &customer_def)?;
        inserter.add_row(&[&"DK-13375", &"Dennis Kane", &518i64, &"Consumer"])?;
        inserter.add_row(&[&"EB-13705", &"Ed Braxton", &815i64, &"Corporate"])?;
        inserter.add_row(&[&"JL-15235", &"Jane Lopez", &2100i64, &"Consumer"])?;
        inserter.add_row(&[&"MK-16790", &"Mike Kelly", &320i64, &"Corporate"])?;
        inserter.execute()?;
    }

    // Insert Orders data
    {
        let mut inserter = Inserter::new(&connection, &orders_def)?;
        // Orders for Dennis Kane
        inserter.add_row(&[
            &399i16,
            &"DK-13375",
            &Date::new(2012, 9, 7),
            &"CA-2011-100006",
            &Date::new(2012, 9, 13),
            &"Standard Class",
        ])?;
        inserter.add_row(&[
            &400i16,
            &"DK-13375",
            &Date::new(2012, 10, 15),
            &"CA-2011-100007",
            &Date::new(2012, 10, 20),
            &"Express Class",
        ])?;
        // Orders for Ed Braxton
        inserter.add_row(&[
            &530i16,
            &"EB-13705",
            &Date::new(2012, 7, 8),
            &"CA-2011-100090",
            &Date::new(2012, 7, 12),
            &"Standard Class",
        ])?;
        // Orders for Jane Lopez
        inserter.add_row(&[
            &601i16,
            &"JL-15235",
            &Date::new(2012, 8, 22),
            &"CA-2011-100150",
            &Date::new(2012, 8, 28),
            &"First Class",
        ])?;
        inserter.execute()?;
    }

    Ok(())
}

fn run_delete_data_in_existing_hyper_file() -> Result<()> {
    println!("EXAMPLE - Delete data from an existing Hyper file");

    // Create test_results directory if it doesn't exist
    std::fs::create_dir_all("test_results")?;

    let path_to_database = "test_results/delete_data_in_existing_hyper_file.hyper";

    {
        use hyperdb_api::Parameters;
        let mut params = Parameters::new();
        params.set("log_dir", "test_results");
        let hyper = HyperProcess::new(None, Some(&params))?;

        // First, create the sample database with test data
        println!("Creating sample database with test data...");
        create_sample_data(&hyper, path_to_database)?;

        // Connect to existing Hyper file
        {
            let connection = Connection::new(&hyper, path_to_database, CreateMode::DoNotCreate)?;

            // Show initial row counts
            println!("\nInitial data:");
            print_row_counts(&connection)?;

            // Delete all rows from Orders where Customer Name is 'Dennis Kane'
            println!(
                "\nDelete all rows from customer with the name 'Dennis Kane' from table {}.",
                escape_name("Orders")?
            );

            // `execute_command` executes a SQL statement and returns the impacted row count.
            let delete_orders_sql = format!(
                "DELETE FROM {} WHERE {} = ANY(SELECT {} FROM {} WHERE {} = {})",
                escape_name("Orders")?,
                escape_name("Customer ID")?,
                escape_name("Customer ID")?,
                escape_name("Customer")?,
                escape_name("Customer Name")?,
                escape_string_literal("Dennis Kane")
            );

            let row_count = connection.execute_command(&delete_orders_sql)?;
            println!(
                "The number of deleted rows in table {} is {}.\n",
                escape_name("Orders")?,
                row_count
            );

            // Delete Dennis Kane from Customer table
            println!(
                "Delete all rows from customer with the name 'Dennis Kane' from table {}.",
                escape_name("Customer")?
            );

            let delete_customer_sql = format!(
                "DELETE FROM {} WHERE {} = {}",
                escape_name("Customer")?,
                escape_name("Customer Name")?,
                escape_string_literal("Dennis Kane")
            );

            let row_count = connection.execute_command(&delete_customer_sql)?;
            println!("The number of deleted rows in table Customer is {row_count}.");

            // Show final row counts
            println!("\nFinal data after deletions:");
            print_row_counts(&connection)?;
        }
        println!("\nThe connection to the Hyper file has been closed.");
    }
    println!("The Hyper Process has been shut down.");

    Ok(())
}

/// Prints row counts for Customer and Orders tables.
fn print_row_counts(connection: &Connection) -> Result<()> {
    for table in &["Customer", "Orders"] {
        let count: Option<i64> = connection
            .execute_scalar_query(&format!("SELECT COUNT(*) FROM {}", escape_name(table)?))?;
        println!("  {} rows in {}", count.unwrap_or(0), table);
    }
    Ok(())
}

fn main() {
    match run_delete_data_in_existing_hyper_file() {
        Ok(()) => {}
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
