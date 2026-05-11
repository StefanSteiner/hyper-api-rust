// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Example: Insert data into multiple tables
//!
//! This example demonstrates how to:
//! - Create a multi-table Hyper file with different column types
//! - Use the high-performance Inserter for bulk data insertion
//! - Insert data into Orders, Customer, Products, and Line Items tables
//!
//! This is a Rust port of the C++ example `insert_data_into_multiple_tables.cpp`.
//!
//! Run with: cargo run -p hyperdb-api --example `insert_data_into_multiple_tables`

use hyperdb_api::{
    Catalog, Connection, CreateMode, Date, HyperProcess, Inserter, Result, SqlType, TableDefinition,
};

/// Creates the Orders table definition.
fn orders_table() -> TableDefinition {
    // Since the table name is not prefixed with an explicit schema name,
    // the table will reside in the default "public" namespace.
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

/// Creates the Products table definition.
fn products_table() -> TableDefinition {
    TableDefinition::new("Products")
        .add_required_column("Category", SqlType::text())
        .add_required_column("Product ID", SqlType::text())
        .add_required_column("Product Name", SqlType::text())
        .add_required_column("Sub-Category", SqlType::text())
}

/// Creates the Line Items table definition.
fn line_items_table() -> TableDefinition {
    TableDefinition::new("Line Items")
        .add_required_column("Line Item ID", SqlType::big_int())
        .add_required_column("Order ID", SqlType::text())
        .add_required_column("Product ID", SqlType::text())
        .add_required_column("Sales", SqlType::double())
        .add_required_column("Quantity", SqlType::small_int())
        .add_nullable_column("Discount", SqlType::double())
        .add_required_column("Profit", SqlType::double())
}

fn run_insert_data_into_multiple_tables() -> Result<()> {
    println!("EXAMPLE - Insert data into multiple tables within a new Hyper file");

    // Create test_results directory if it doesn't exist
    std::fs::create_dir_all("test_results")?;

    let path_to_database = "test_results/insert_data_into_multiple_tables.hyper";

    let endpoint = {
        use hyperdb_api::Parameters;
        let mut params = Parameters::new();
        params.set("log_dir", "test_results");
        let hyper = HyperProcess::new(None, Some(&params))?;
        let endpoint = hyper.endpoint().unwrap().to_string();
        println!("Hyper process started at endpoint: {endpoint}");

        // Creates new Hyper file "superstore.hyper"
        // Replaces existing file with CreateMode::CreateAndReplace if it already exists.
        {
            let connection =
                Connection::new(&hyper, path_to_database, CreateMode::CreateAndReplace)?;
            let catalog = Catalog::new(&connection);

            // Create table definitions
            let orders_def = orders_table();
            let customer_def = customer_table();
            let products_def = products_table();
            let line_items_def = line_items_table();

            // Create multiple tables
            catalog.create_table(&orders_def)?;
            catalog.create_table(&customer_def)?;
            catalog.create_table(&products_def)?;
            catalog.create_table(&line_items_def)?;

            // Insert data into Orders table
            {
                let mut inserter = Inserter::new(&connection, &orders_def)?;
                inserter.add_row(&[
                    &399i16,
                    &"DK-13375",
                    &Date::new(2012, 9, 7),
                    &"CA-2011-100006",
                    &Date::new(2012, 9, 13),
                    &"Standard Class",
                ])?;
                inserter.add_row(&[
                    &530i16,
                    &"EB-13705",
                    &Date::new(2012, 7, 8),
                    &"CA-2011-100090",
                    &Date::new(2012, 7, 12),
                    &"Standard Class",
                ])?;
                inserter.execute()?;
            }

            // Insert data into Customer table
            {
                let mut inserter = Inserter::new(&connection, &customer_def)?;
                inserter.add_row(&[&"DK-13375", &"Dennis Kane", &518i64, &"Consumer"])?;
                inserter.add_row(&[&"EB-13705", &"Ed Braxton", &815i64, &"Corporate"])?;
                inserter.execute()?;
            }

            // Insert individual row into Products table
            {
                let mut inserter = Inserter::new(&connection, &products_def)?;
                inserter.add_row(&[
                    &"Technology",
                    &"TEC-PH-10002075",
                    &"AT&T EL51110 DECT",
                    &"Phones",
                ])?;
                inserter.execute()?;
            }

            // Insert data into Line Items table
            {
                let mut inserter = Inserter::new(&connection, &line_items_def)?;
                inserter.add_row(&[
                    &2718i64,
                    &"CA-2011-100006",
                    &"TEC-PH-10002075",
                    &377.97f64,
                    &3i16,
                    &0.0f64,
                    &109.6113f64,
                ])?;
                // Second row with NULL discount
                inserter.add_i64(2719)?;
                inserter.add_str("CA-2011-100090")?;
                inserter.add_str("TEC-PH-10002075")?;
                inserter.add_f64(377.97)?;
                inserter.add_i16(3)?;
                inserter.add_null()?; // NULL discount
                inserter.add_f64(109.6113)?;
                inserter.end_row()?;
                inserter.execute()?;
            }

            // Print row counts for each table using generic execute_scalar_query<T>
            for table_def in [&orders_def, &customer_def, &products_def, &line_items_def] {
                let row_count: Option<i64> = connection.execute_scalar_query(&format!(
                    "SELECT COUNT(*) FROM {}",
                    table_def.qualified_name()
                ))?;
                println!(
                    "The number of rows in table {} is {}.",
                    table_def.qualified_name(),
                    row_count.unwrap_or(0)
                );
            }
        }
        println!("The connection to the Hyper file has been closed.");
        endpoint // Return endpoint before hyper is dropped
    };

    // HyperProcess is automatically shut down when dropped
    println!("The Hyper Process has been shut down.");

    // Verify that the endpoint no longer works after shutdown
    println!("\nVerifying that the endpoint is no longer accessible after shutdown...");
    match Connection::connect(&endpoint, path_to_database, CreateMode::CreateIfNotExists) {
        Ok(_) => {
            eprintln!(
                "ERROR: Connection succeeded unexpectedly - Hyper process should be shut down!"
            );
            return Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                "Endpoint should not be accessible after Hyper shutdown",
            )
            .into());
        }
        Err(e) => {
            println!("[OK] Verified: Endpoint is no longer accessible (expected error: {e})");
        }
    }

    Ok(())
}

fn main() {
    match run_insert_data_into_multiple_tables() {
        Ok(()) => {}
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
