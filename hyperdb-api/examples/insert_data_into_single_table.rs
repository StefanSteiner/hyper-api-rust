// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Example: Insert data into a single table
//!
//! This example demonstrates how to:
//! - Create a single-table Hyper file with different column types
//! - Create a custom schema
//! - Use the high-performance Inserter for bulk data insertion
//! - List tables in a schema
//!
//! This is a Rust port of the C++ example `insert_data_into_single_table.cpp`.
//!
//! Run with: cargo run -p hyperdb-api --example `insert_data_into_single_table`

use hyperdb_api::{
    Catalog, Connection, CreateMode, HyperProcess, Inserter, Result, SqlType, TableDefinition,
};

/// Creates the Extract table definition.
/// The table is called "Extract" and will be created in the "Extract" schema.
/// This has historically been the default table name and schema for extracts created by Tableau.
fn extract_table() -> TableDefinition {
    TableDefinition::from("Extract")
        .with_schema("Extract")
        .add_required_column("Customer ID", SqlType::text())
        .add_required_column("Customer Name", SqlType::text())
        .add_required_column("Loyalty Reward Points", SqlType::big_int())
        .add_required_column("Segment", SqlType::text())
}

fn run_insert_data_into_single_table() -> Result<()> {
    println!("EXAMPLE - Insert data into a single table within a new Hyper file");

    // Create test_results directory if it doesn't exist
    std::fs::create_dir_all("test_results")?;

    let path_to_database = "test_results/insert_data_into_single_table.hyper";

    {
        use hyperdb_api::Parameters;
        let mut params = Parameters::new();
        params.set("log_dir", "test_results");
        let hyper = HyperProcess::new(None, Some(&params))?;

        // Creates new Hyper file "customer.hyper".
        // Replaces existing file if it already exists.
        {
            let connection =
                Connection::new(&hyper, path_to_database, CreateMode::CreateAndReplace)?;
            let catalog = Catalog::new(&connection);

            // Create the schema and the table.
            let extract_def = extract_table();
            catalog.create_schema("Extract")?;
            catalog.create_table(&extract_def)?;

            // Insert data into the "Extract"."Extract" table.
            {
                let mut inserter = Inserter::new(&connection, &extract_def)?;
                inserter.add_row(&[&"DK-13375", &"Dennis Kane", &518i64, &"Consumer"])?;
                inserter.add_row(&[&"EB-13705", &"Ed Braxton", &815i64, &"Corporate"])?;
                inserter.execute()?;
            }

            // Print the table names in the "Extract" schema.
            let table_names = catalog.get_table_names("Extract")?;
            print!("Tables available in {path_to_database} in the Extract schema are: ");
            for table_name in &table_names {
                print!("{table_name}\t");
            }
            println!();

            // Number of rows in the "Extract"."Extract" table.
            // Using generic execute_scalar_query<T> (similar to C++ executeScalarQuery<T>).
            let row_count: Option<i64> = connection.execute_scalar_query(&format!(
                "SELECT COUNT(*) FROM {}",
                extract_def.qualified_name()
            ))?;
            println!(
                "The number of rows in table {} is {}.",
                extract_def.qualified_name(),
                row_count.unwrap_or(0)
            );
        }
        println!("The connection to the Hyper file has been closed.");
    }
    println!("The Hyper Process has been shut down.");

    Ok(())
}

fn main() {
    match run_insert_data_into_single_table() {
        Ok(()) => {}
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
