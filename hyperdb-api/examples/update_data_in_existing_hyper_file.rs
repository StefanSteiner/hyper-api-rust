// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Example: Update data in an existing Hyper file
//!
//! This example demonstrates how to:
//! - Connect to an existing Hyper file
//! - Update data using SQL UPDATE statements
//! - Query data before and after updates
//!
//! This is a Rust port of the C++ example `update_data_in_existing_hyper_file.cpp`.
//!
//! Run with: cargo run -p hyperdb-api --example `update_data_in_existing_hyper_file`

use hyperdb_api::{
    escape_name, escape_string_literal, Catalog, Connection, CreateMode, HyperProcess, Inserter,
    Result, SqlType, TableDefinition,
};

/// Creates the Customer table definition.
fn customer_table() -> TableDefinition {
    TableDefinition::new("Customer")
        .add_required_column("Customer ID", SqlType::text())
        .add_required_column("Customer Name", SqlType::text())
        .add_required_column("Loyalty Reward Points", SqlType::big_int())
        .add_required_column("Segment", SqlType::text())
}

/// Creates a sample Hyper file with Customer data.
fn create_sample_data(hyper: &HyperProcess, path: &str) -> Result<()> {
    // Create the data directory if it doesn't exist
    if let Some(parent) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(parent)?;
    }

    let connection = Connection::new(hyper, path, CreateMode::CreateAndReplace)?;
    let catalog = Catalog::new(&connection);

    let customer_def = customer_table();
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

    Ok(())
}

fn run_update_data_in_existing_hyper_file() -> Result<()> {
    println!("EXAMPLE - Update existing data in a Hyper file");

    // Create test_results directory if it doesn't exist
    std::fs::create_dir_all("test_results")?;

    let path_to_database = "test_results/update_data_in_existing_hyper_file.hyper";

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

            // Print pre-update data
            println!("\nPre-Update: Individual rows showing 'Loyalty Reward Points' and 'Segment' columns:");
            print_customer_data(&connection)?;

            // Update 'Customers' table by adding 50 Loyalty Reward Points to all Corporate Customers.
            println!("\nUpdate 'Customer' table by adding 50 Loyalty Reward Points to all Corporate Customers.");

            let update_sql = format!(
                "UPDATE {} SET {} = {} + 50 WHERE {} = {}",
                escape_name("Customer")?,
                escape_name("Loyalty Reward Points")?,
                escape_name("Loyalty Reward Points")?,
                escape_name("Segment")?,
                escape_string_literal("Corporate")
            );

            let row_count = connection.execute_command(&update_sql)?;
            println!(
                "The number of updated rows in table {} is {}.",
                escape_name("Customer")?,
                row_count
            );

            // Print post-update data
            println!("\nPost-Update: Individual rows showing 'Loyalty Reward Points' and 'Segment' columns:");
            print_customer_data(&connection)?;
        }
        println!("\nThe connection to the Hyper file has been closed.");
    }
    println!("The Hyper Process has been shut down.");

    Ok(())
}

/// Prints Loyalty Reward Points and Segment columns from Customer table.
fn print_customer_data(connection: &Connection) -> Result<()> {
    let query = format!(
        "SELECT {}, {} FROM {}",
        escape_name("Loyalty Reward Points")?,
        escape_name("Segment")?,
        escape_name("Customer")?
    );

    let mut result = connection.execute_query(&query)?;
    while let Some(chunk) = result.next_chunk()? {
        for row in &chunk {
            let loyalty_points: i64 = row.get(0).unwrap_or(0);
            let segment: String = row.get(1).unwrap_or_default();
            println!("  {loyalty_points}\t{segment}");
        }
    }

    Ok(())
}

fn main() {
    match run_update_data_in_existing_hyper_file() {
        Ok(()) => {}
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
