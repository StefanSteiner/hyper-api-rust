// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Example: Read and print data from an existing Hyper file
//!
//! This example demonstrates how to:
//! - Connect to an existing Hyper file
//! - List schemas and tables
//! - Get table definitions (column names, types, nullability)
//! - Execute queries and print results
//!
//! This is a Rust port of the C++ example `read_and_print_data_from_existing_hyper_file.cpp`.
//!
//! Run with: cargo run -p hyperdb-api --example `read_and_print_data_from_existing_hyper_file`

use hyperdb_api::{
    Catalog, Connection, CreateMode, HyperProcess, Inserter, Result, SqlType, TableDefinition,
    TableName,
};

/// Creates the Extract table definition.
/// The table is called "Extract" and will be created in the "Extract" schema.
/// This has historically been the default table name and schema for extracts created by Tableau.
fn extract_table() -> TableDefinition {
    TableDefinition::new("Extract")
        .with_schema("Extract")
        .add_required_column("Customer ID", SqlType::text())
        .add_required_column("Customer Name", SqlType::text())
        .add_required_column("Loyalty Reward Points", SqlType::big_int())
        .add_required_column("Segment", SqlType::text())
}

/// Creates a sample Hyper file with Extract data.
fn create_sample_data(hyper: &HyperProcess, path: &str) -> Result<()> {
    // Create the data directory if it doesn't exist
    if let Some(parent) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(parent)?;
    }

    let connection = Connection::new(hyper, path, CreateMode::CreateAndReplace)?;
    let catalog = Catalog::new(&connection);

    // Create schema and table
    let extract_def = extract_table();
    catalog.create_schema("Extract")?;
    catalog.create_table(&extract_def)?;

    // Insert data
    {
        let mut inserter = Inserter::new(&connection, &extract_def)?;
        inserter.add_row(&[&"DK-13375", &"Dennis Kane", &518i64, &"Consumer"])?;
        inserter.add_row(&[&"EB-13705", &"Ed Braxton", &815i64, &"Corporate"])?;
        inserter.add_row(&[&"JL-15235", &"Jane Lopez", &2100i64, &"Consumer"])?;
        inserter.add_row(&[&"MK-16790", &"Mike Kelly", &320i64, &"Corporate"])?;
        inserter.add_row(&[&"SR-20815", &"Sarah Roberts", &1450i64, &"Home Office"])?;
        inserter.execute()?;
    }

    Ok(())
}

fn run_read_and_print_data_from_existing_hyper_file() -> Result<()> {
    println!("EXAMPLE - Read data from an existing Hyper file");

    // Create test_results directory if it doesn't exist
    std::fs::create_dir_all("test_results")?;

    let path_to_database = "test_results/read_and_print_data_from_existing_hyper_file.hyper";

    {
        use hyperdb_api::Parameters;
        let mut params = Parameters::new();
        params.set("log_dir", "test_results");
        let hyper = HyperProcess::new(None, Some(&params))?;

        // First, create the sample database with test data
        println!("Creating sample database with test data...\n");
        create_sample_data(&hyper, path_to_database)?;

        // Connect to existing Hyper file
        {
            let connection = Connection::new(&hyper, path_to_database, CreateMode::DoNotCreate)?;
            let catalog = Catalog::new(&connection);

            // The table names in the "Extract" schema.
            let table_names = catalog.get_table_names("Extract")?;

            for table_name in &table_names {
                let table_definition =
                    catalog.get_table_definition(format!("Extract.{table_name}"))?;

                println!(
                    "Table {} has qualified name: {}",
                    table_name,
                    table_definition.qualified_name()
                );

                for column in table_definition.columns() {
                    let nullability = if column.nullable {
                        "Nullable"
                    } else {
                        "NotNullable"
                    };
                    println!(
                        "\t Column {} has type {} and nullability {}",
                        column.name,
                        column.type_name(),
                        nullability
                    );
                }
                println!();
            }

            // Print all rows from the "Extract"."Extract" table.
            // Demonstrating FromStr parsing: parse table name from string format using .parse()
            let extract_table_name: TableName = "Extract.Extract".parse()?;
            println!("Parsed table name using .parse(): {extract_table_name}");
            // Create TableDefinition using the parsed table name parts
            let extract_table = TableDefinition::new(extract_table_name.unescaped())
                .with_schema(extract_table_name.schema().unwrap().unescaped());
            println!(
                "These are all rows in the table {}:",
                extract_table.qualified_name()
            );

            let query = format!("SELECT * FROM {}", extract_table.qualified_name());
            let mut result = connection.execute_query(&query)?;

            while let Some(chunk) = result.next_chunk()? {
                for row in &chunk {
                    // Print each column value separated by tabs
                    let customer_id: String = row.get(0).unwrap_or_default();
                    let customer_name: String = row.get(1).unwrap_or_default();
                    let loyalty_points: i64 = row.get(2).unwrap_or(0);
                    let segment: String = row.get(3).unwrap_or_default();

                    println!("{customer_id}\t{customer_name}\t{loyalty_points}\t{segment}");
                }
            }
        }
        println!("\nThe connection to the Hyper file has been closed.");
    }
    println!("The Hyper Process has been shut down.");

    Ok(())
}

fn main() {
    match run_read_and_print_data_from_existing_hyper_file() {
        Ok(()) => {}
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
