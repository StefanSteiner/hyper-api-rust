// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Example: Insert geospatial data into a Hyper file
//!
//! This example demonstrates how to:
//! - Create a table with a geography column
//! - Insert geospatial data using WKT (Well-Known Text) format
//! - Use SQL CAST expressions to convert text to geography type
//! - Use column mappings for data transformation
//!
//! This is a Rust port of the C++ example `insert_geospatial_data_to_a_hyper_file.cpp`.
//!
//! Run with: cargo run -p hyperdb-api --example `insert_geospatial_data_to_a_hyper_file`

use hyperdb_api::{
    escape_name, Catalog, ColumnMapping, Connection, CreateMode, HyperProcess, Inserter, Result,
    SqlType, TableDefinition,
};

/// Creates the Extract table definition (target table).
/// The table is called "Extract" and will be created in the "Extract" schema.
/// This has historically been the default table name and schema for extracts created by Tableau.
fn extract_table() -> TableDefinition {
    TableDefinition::new("Extract")
        .with_schema("Extract")
        .add_required_column("Name", SqlType::text())
        .add_required_column("Location", SqlType::tabgeography())
}

/// Creates the inserter definition (defines input columns).
/// The data input has two text values: Name and `Location_as_text`.
fn inserter_definition() -> TableDefinition {
    TableDefinition::new("_inserter_input")
        .add_required_column("Name", SqlType::text())
        .add_required_column("Location_as_text", SqlType::text())
}

fn run_insert_spatial_data_to_a_hyper_file() -> Result<()> {
    println!("EXAMPLE - Insert geospatial data into a single table within a new Hyper file");

    // Create test_results directory if it doesn't exist
    std::fs::create_dir_all("test_results")?;

    let path_to_database = "test_results/insert_geospatial_data_to_a_hyper_file.hyper";

    {
        use hyperdb_api::Parameters;
        let mut params = Parameters::new();
        params.set("log_dir", "test_results");
        let hyper = HyperProcess::new(None, Some(&params))?;

        // Creates new Hyper file "spatial_data.hyper".
        // Replaces existing file with CreateMode::CreateAndReplace if it already exists.
        {
            let connection =
                Connection::new(&hyper, path_to_database, CreateMode::CreateAndReplace)?;
            let catalog = Catalog::new(&connection);

            // Create the schema and the table.
            let extract_def = extract_table();
            catalog.create_schema("Extract")?;
            catalog.create_table(&extract_def)?;

            // Hyper API's Inserter allows users to transform data during insertion.
            // To make use of data transformation during insertion, the inserter requires:
            //   1. The connection to the Hyper instance containing the table
            //   2. The table name or table definition into which data is inserted
            //   3. List of ColumnMapping
            //      This list informs the inserter how each column in the target table is transformed.
            //      ColumnMapping maps a valid SQL expression (if any) to a column in the target table.
            //   4. Inserter Definition, a list of column definitions for all the input values provided

            // Column 'Name' is inserted into "Extract"."Extract" as-is.
            // Column 'Location' in "Extract"."Extract" of `tableau.tabgeography` type is computed
            // from Column 'Location_as_text' of `text` type using the expression
            // 'CAST("Location_as_text" AS TABLEAU.TABGEOGRAPHY)'.

            let text_to_geography_cast_expression = format!(
                "CAST({} AS TABLEAU.TABGEOGRAPHY)",
                escape_name("Location_as_text")?
            );

            let column_mappings = vec![
                ColumnMapping::new("Name"),
                ColumnMapping::with_expression("Location", text_to_geography_cast_expression),
            ];

            // Get the inserter definition
            let inserter_def = inserter_definition();

            // Insert geospatial data into the "Extract"."Extract" table using CAST expression.
            {
                let mut inserter = Inserter::with_column_mappings(
                    &connection,
                    &inserter_def,
                    extract_def.qualified_name(),
                    &column_mappings,
                )?;
                // Insert locations using WKT (Well-Known Text) format: point(longitude latitude)
                inserter.add_row(&[&"Seattle", &"point(-122.338083 47.647528)"])?;
                inserter.add_row(&[&"Munich", &"point(11.584329 48.139257)"])?;
                inserter.execute()?;
            }

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
        println!("\nThe connection to the Hyper file has been closed.");
    }
    println!("The Hyper Process has been shut down.");

    Ok(())
}

fn main() {
    match run_insert_spatial_data_to_a_hyper_file() {
        Ok(()) => {}
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
