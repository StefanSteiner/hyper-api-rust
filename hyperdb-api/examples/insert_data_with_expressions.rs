// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Example: Insert data with expressions
//!
//! This example demonstrates how to:
//! - Push down computations to Hyper during data insertion using expressions
//! - Transform data types during insertion (e.g., text to timestamp)
//! - Use CASE expressions to compute values
//! - Use column mappings with SQL expressions
//!
//! This is a Rust port of the C++ example `insert_data_with_expressions.cpp`.
//!
//! Run with: cargo run -p hyperdb-api --example `insert_data_with_expressions`

use hyperdb_api::{
    escape_name, escape_string_literal, Catalog, ColumnMapping, Connection, CreateMode,
    HyperProcess, Inserter, Result, SqlType, TableDefinition,
};

/// Creates the Extract table definition (target table).
/// The table is called "Extract" and will be created in the "Extract" schema.
/// This has historically been the default table name and schema for extracts created by Tableau.
fn extract_table() -> TableDefinition {
    TableDefinition::new("Extract")
        .with_schema("Extract")
        .add_required_column("Order ID", SqlType::int())
        .add_required_column("Ship Timestamp", SqlType::timestamp())
        .add_required_column("Ship Mode", SqlType::text())
        .add_required_column("Ship Priority", SqlType::int())
}

/// Creates the inserter definition (defines input columns).
/// These are the columns we'll actually insert values for.
fn inserter_definition() -> TableDefinition {
    TableDefinition::new("_inserter_input")
        .add_required_column("Order ID", SqlType::int())
        .add_required_column("Ship Timestamp Text", SqlType::text())
        .add_required_column("Ship Mode", SqlType::text())
        .add_required_column("Ship Priority Text", SqlType::text())
}

fn run_insert_data_with_expressions() -> Result<()> {
    println!("EXAMPLE - Push down computations to Hyper during data insertion using expressions");

    // Create test_results directory if it doesn't exist
    std::fs::create_dir_all("test_results")?;

    let path_to_database = "test_results/insert_data_with_expressions.hyper";

    {
        use hyperdb_api::Parameters;
        let mut params = Parameters::new();
        params.set("log_dir", "test_results");
        let hyper = HyperProcess::new(None, Some(&params))?;

        // Creates new Hyper file "orders.hyper".
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
            //      For example: ColumnMapping::with_expression("target_column", "colA * colB")
            //      SQL expression is optional - for a column without transformation, just use ColumnMapping::new("column")
            //   4. Inserter Definition, a list of column definitions for all the input values provided

            // Column 'Order Id' is inserted into "Extract"."Extract" as-is.
            // Column 'Ship Timestamp' in "Extract"."Extract" of timestamp type is computed from
            //   Column 'Ship Timestamp Text' of text type using 'to_timestamp()'.
            // Column 'Ship Mode' is inserted into "Extract"."Extract" as-is.
            // Column 'Ship Priority' in "Extract"."Extract" of integer type is computed from
            //   Column 'Ship Priority Text' of text type using 'CASE' statement.

            let text_to_timestamp_expression = format!(
                "to_timestamp({}, {})",
                escape_name("Ship Timestamp Text")?,
                escape_string_literal("YYYY-MM-DD HH24:MI:SS")
            );

            let ship_priority_as_int_case_expression = format!(
                "CASE {} WHEN {} THEN 1 WHEN {} THEN 2 WHEN {} THEN 3 END",
                escape_name("Ship Priority Text")?,
                escape_string_literal("Urgent"),
                escape_string_literal("Medium"),
                escape_string_literal("Low")
            );

            let column_mappings = vec![
                ColumnMapping::new("Order ID"),
                ColumnMapping::with_expression("Ship Timestamp", text_to_timestamp_expression),
                ColumnMapping::new("Ship Mode"),
                ColumnMapping::with_expression(
                    "Ship Priority",
                    ship_priority_as_int_case_expression,
                ),
            ];

            // Get the inserter definition
            let inserter_def = inserter_definition();

            // Insert data into the "Extract"."Extract" table using expressions.
            {
                let mut inserter = Inserter::with_column_mappings(
                    &connection,
                    &inserter_def,
                    extract_def.qualified_name(),
                    &column_mappings,
                )?;
                inserter.add_row(&[
                    &399i32,
                    &"2012-09-13 10:00:00",
                    &"Express Class",
                    &"Urgent",
                ])?;
                inserter.add_row(&[&530i32, &"2012-07-12 14:00:00", &"Standard Class", &"Low"])?;
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

            // Print the inserted data to verify the transformations
            println!("\nData in table {}:", extract_def.qualified_name());
            let query = format!(
                "SELECT \"Order ID\", \"Ship Timestamp\", \"Ship Mode\", \"Ship Priority\" FROM {}",
                extract_def.qualified_name()
            );
            let mut result = connection.execute_query(&query)?;
            while let Some(chunk) = result.next_chunk()? {
                for row in &chunk {
                    let order_id = row.get_i32(0).unwrap_or(0);
                    let ship_timestamp = row.get::<String>(1).unwrap_or_default();
                    let ship_mode = row.get::<String>(2).unwrap_or_default();
                    let ship_priority = row.get_i32(3).unwrap_or(0);
                    println!(
                        "  Order {order_id}: Timestamp={ship_timestamp}, Mode={ship_mode}, Priority={ship_priority}"
                    );
                }
            }
        }
        println!("\nThe connection to the Hyper file has been closed.");
    }
    println!("The Hyper Process has been shut down.");

    Ok(())
}

fn main() {
    match run_insert_data_with_expressions() {
        Ok(()) => {}
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
