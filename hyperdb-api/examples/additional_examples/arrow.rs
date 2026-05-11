// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Example: Arrow IPC stream data insertion and query results
//!
//! This example demonstrates how to use Arrow IPC format for both:
//! - Inserting data using `ArrowInserter`
//! - Reading query results using `execute_query_to_arrow()`
//!
//! Run with: cargo run -p hyperdb-api --example arrow

#![allow(clippy::cast_precision_loss, reason = "example throughput display")]
// Example harness: intentional wide→narrow conversions for RecordBatch counts,
// synthetic row IDs, and byte-level arithmetic on small demo buffers.
#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    reason = "example harness: demo-sized counts narrow by construction"
)]

use std::sync::Arc;

use arrow::array::{Array, Float64Array, Int32Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::ipc::reader::StreamReader;
use arrow::ipc::writer::StreamWriter;
use arrow::record_batch::RecordBatch;

use hyperdb_api::{
    ArrowInserter, ArrowReader, Catalog, Connection, CreateMode, HyperProcess, Result, SqlType,
    TableDefinition,
};

fn main() -> Result<()> {
    println!("=== Arrow Example ===\n");

    // Create test_results directory if it doesn't exist
    std::fs::create_dir_all("test_results")?;

    // Start Hyper process with logs in test_results
    println!("Starting Hyper process...");
    use hyperdb_api::Parameters;
    let mut params = Parameters::new();
    params.set("log_dir", "test_results");
    let hyper = HyperProcess::new(None, Some(&params))?;

    let db_path = "test_results/arrow.hyper";
    let connection = Connection::new(&hyper, db_path, CreateMode::CreateAndReplace)?;
    println!("Created database: {db_path}\n");

    // Example 1: Basic ArrowInserter with real Arrow data
    example_basic_arrow_insert(&connection)?;

    // Example 2: Multi-batch insertion
    example_multi_batch_insert(&connection)?;

    // Example 3: Using from_table()
    example_from_table(&connection)?;

    // Example 4: Large dataset
    example_large_dataset(&connection)?;

    // Example 5: Arrow roundtrip - write data and read it back as Arrow
    example_arrow_roundtrip(&connection)?;

    // Example 6: ArrowReader convenience methods
    example_arrow_reader(&connection)?;

    // Example 7: Struct mapping from Arrow data
    example_struct_mapping_from_arrow(&connection)?;

    // Print database file size
    if let Ok(metadata) = std::fs::metadata(db_path) {
        let size_bytes = metadata.len();
        let size_mb = size_bytes as f64 / (1024.0 * 1024.0);
        println!("\nDatabase file size: {size_mb:.2} MB ({size_bytes} bytes)");
    }

    println!("\nAll examples completed successfully!");

    Ok(())
}

/// Example 1: Basic `ArrowInserter` with real Arrow data
fn example_basic_arrow_insert(connection: &Connection) -> Result<()> {
    println!("=== Example 1: Basic Arrow Insert ===");

    // Create a table with matching schema
    let catalog = Catalog::new(connection);
    let table_def = TableDefinition::new("arrow_data")
        .add_required_column("id", SqlType::int())
        .add_nullable_column("value", SqlType::double())
        .add_nullable_column("name", SqlType::text());

    catalog.create_table(&table_def)?;
    println!("Created table 'arrow_data'");

    // Generate Arrow IPC data
    let arrow_data = generate_sample_arrow_data();
    println!("Generated {} bytes of Arrow IPC data", arrow_data.len());

    // Insert using ArrowInserter
    let mut inserter = ArrowInserter::new(connection, &table_def)?;
    inserter.insert_data(&arrow_data)?;
    let rows = inserter.execute()?;
    println!("Inserted {rows} rows via Arrow format");

    // Verify the data
    println!("\nVerifying inserted data:");
    let mut result =
        connection.execute_query("SELECT id, value, name FROM arrow_data ORDER BY id")?;
    if let Some(chunk) = result.next_chunk()? {
        for row in &chunk {
            let id: Option<i32> = row.get(0);
            let value: Option<f64> = row.get(1);
            let name: Option<String> = row.get(2);
            println!("  id={id:?}, value={value:?}, name={name:?}");
        }
    }

    println!();
    Ok(())
}

/// Example 2: Multi-batch insertion
fn example_multi_batch_insert(connection: &Connection) -> Result<()> {
    println!("=== Example 2: Multi-Batch Insert ===");

    // Create a table
    let catalog = Catalog::new(connection);
    let table_def = TableDefinition::new("multi_batch_data")
        .add_required_column("id", SqlType::int())
        .add_nullable_column("category", SqlType::text());

    catalog.create_table(&table_def)?;
    println!("Created table 'multi_batch_data'");

    // Generate Arrow IPC data with multiple batches in one stream
    let arrow_data = generate_multi_batch_arrow_data();
    println!(
        "Generated {} bytes of Arrow IPC data with multiple batches",
        arrow_data.len()
    );

    // Insert using ArrowInserter
    let mut inserter = ArrowInserter::new(connection, &table_def)?;
    inserter.insert_data(&arrow_data)?;
    let rows = inserter.execute()?;
    println!("Inserted {rows} rows via Arrow format");

    // Verify count
    let count = connection
        .execute_scalar_query::<i64>("SELECT COUNT(*) FROM multi_batch_data")?
        .unwrap_or(0);
    println!("Verified row count: {count}");

    println!();
    Ok(())
}

/// Example 3: Using `from_table()`
fn example_from_table(connection: &Connection) -> Result<()> {
    println!("=== Example 3: Create Inserter from Existing Table ===");

    // Create a table first
    connection.execute_command(
        "CREATE TABLE IF NOT EXISTS existing_table (
            id INTEGER NOT NULL,
            data TEXT,
            value DOUBLE PRECISION
        )",
    )?;
    println!("Created table 'existing_table'");

    // Create ArrowInserter from the existing table (queries schema from database)
    let mut inserter = ArrowInserter::from_table(connection, "existing_table")?;
    println!("Created ArrowInserter from existing table");

    // Generate matching Arrow data
    let arrow_data = generate_existing_table_arrow_data();
    inserter.insert_data(&arrow_data)?;
    let rows = inserter.execute()?;
    println!("Inserted {rows} rows");

    // Verify
    let count = connection
        .execute_scalar_query::<i64>("SELECT COUNT(*) FROM existing_table")?
        .unwrap_or(0);
    println!("Verified row count: {count}");

    println!();
    Ok(())
}

/// Example 4: Large dataset insertion using streaming `insert_batch()`
///
/// Each `RecordBatch` is serialized and sent immediately — memory usage stays
/// at `O(batch_size)` regardless of the total row count.
fn example_large_dataset(connection: &Connection) -> Result<()> {
    println!("=== Example 4: Large Dataset (Streaming insert_batch) ===");

    // Create a table
    let catalog = Catalog::new(connection);
    let table_def = TableDefinition::new("large_data")
        .add_required_column("id", SqlType::int())
        .add_nullable_column("value", SqlType::double())
        .add_nullable_column("text", SqlType::text());

    catalog.create_table(&table_def)?;
    println!("Created table 'large_data'");

    let row_count = 10_000_000usize;
    let chunk_size = 100_000usize;
    let schema = Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("value", DataType::Float64, true),
        Field::new("text", DataType::Utf8, true),
    ]);

    let start = std::time::Instant::now();
    let mut inserter = ArrowInserter::new(connection, &table_def)?;

    // Stream batches one at a time — each is serialized and sent immediately.
    // No RecordBatches accumulate in memory.
    for chunk_start in (0..row_count).step_by(chunk_size) {
        let chunk_end = (chunk_start + chunk_size).min(row_count);
        let ids: Vec<i32> = (chunk_start..chunk_end).map(|i| i as i32).collect();
        let values: Vec<Option<f64>> = (chunk_start..chunk_end)
            .map(|i| Some(i as f64 * 0.1))
            .collect();
        let texts: Vec<Option<String>> = (chunk_start..chunk_end)
            .map(|i| Some(generate_random_string(i, 32)))
            .collect();

        let batch = RecordBatch::try_new(
            Arc::new(schema.clone()),
            vec![
                Arc::new(Int32Array::from(ids)),
                Arc::new(Float64Array::from(values)),
                Arc::new(StringArray::from(texts)),
            ],
        )
        .expect("Failed to create record batch");

        inserter.insert_batch(&batch)?;
    }

    let rows = inserter.execute()?;
    let elapsed = start.elapsed();

    println!(
        "Inserted {} rows in {:?} ({:.0} rows/sec)",
        rows,
        elapsed,
        rows as f64 / elapsed.as_secs_f64()
    );

    // Verify
    let count = connection
        .execute_scalar_query::<i64>("SELECT COUNT(*) FROM large_data")?
        .unwrap_or(0);
    let sum = connection
        .execute_scalar_query::<i64>("SELECT SUM(id) FROM large_data")?
        .unwrap_or(0);
    println!("Verified: {count} rows, sum(id) = {sum}");

    println!();
    Ok(())
}

/// Example 5: Arrow roundtrip - write data and read it back as Arrow, then verify
fn example_arrow_roundtrip(connection: &Connection) -> Result<()> {
    println!("=== Example 5: Arrow Roundtrip (Write & Read Back) ===");

    // Create a table
    let catalog = Catalog::new(connection);
    let table_def = TableDefinition::new("roundtrip_data")
        .add_required_column("id", SqlType::int())
        .add_nullable_column("value", SqlType::double())
        .add_nullable_column("name", SqlType::text());

    catalog.create_table(&table_def)?;
    println!("Created table 'roundtrip_data'");

    // Create original data
    let schema = Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("value", DataType::Float64, true),
        Field::new("name", DataType::Utf8, true),
    ]);

    let original_ids = vec![1, 2, 3, 4, 5];
    let original_values = vec![Some(1.5), Some(2.5), None, Some(4.5), Some(5.5)];
    let original_names = vec![
        Some("Alice"),
        Some("Bob"),
        Some("Charlie"),
        None,
        Some("Eve"),
    ];

    let original_batch = RecordBatch::try_new(
        Arc::new(schema.clone()),
        vec![
            Arc::new(Int32Array::from(original_ids.clone())),
            Arc::new(Float64Array::from(original_values.clone())),
            Arc::new(StringArray::from(original_names.clone())),
        ],
    )
    .expect("Failed to create original batch");

    // Write to Arrow IPC stream
    let arrow_data = write_arrow_ipc_stream(&schema, &[original_batch]);
    println!(
        "Generated {} bytes of Arrow IPC data with {} rows",
        arrow_data.len(),
        original_ids.len()
    );

    // Insert into Hyper using ArrowInserter
    let mut inserter = ArrowInserter::new(connection, &table_def)?;
    inserter.insert_data(&arrow_data)?;
    let rows_inserted = inserter.execute()?;
    println!("Inserted {rows_inserted} rows via Arrow format");

    // Read back as Arrow IPC stream
    let result_arrow_data = connection
        .execute_query_to_arrow("SELECT id, value, name FROM roundtrip_data ORDER BY id")?;
    println!(
        "Read back {} bytes of Arrow IPC data",
        result_arrow_data.len()
    );

    // Parse the Arrow IPC stream
    let cursor = std::io::Cursor::new(&result_arrow_data);
    let reader = StreamReader::try_new(cursor, None).expect("Failed to create Arrow reader");

    let mut total_rows = 0;
    let mut all_ids: Vec<i32> = Vec::new();
    let mut all_values: Vec<Option<f64>> = Vec::new();
    let mut all_names: Vec<Option<String>> = Vec::new();

    for batch_result in reader {
        let batch = batch_result.expect("Failed to read batch");
        total_rows += batch.num_rows();

        // Extract columns
        let id_array = batch
            .column(0)
            .as_any()
            .downcast_ref::<Int32Array>()
            .expect("Expected Int32Array");
        let value_array = batch
            .column(1)
            .as_any()
            .downcast_ref::<Float64Array>()
            .expect("Expected Float64Array");
        let name_array = batch
            .column(2)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("Expected StringArray");

        for i in 0..batch.num_rows() {
            all_ids.push(id_array.value(i));
            all_values.push(if value_array.is_null(i) {
                None
            } else {
                Some(value_array.value(i))
            });
            all_names.push(if name_array.is_null(i) {
                None
            } else {
                Some(name_array.value(i).to_string())
            });
        }
    }

    println!("Parsed {total_rows} rows from Arrow response");

    // Verify the data matches
    let mut all_match = true;
    for (i, &original_id) in original_ids.iter().enumerate() {
        let matches = all_ids[i] == original_id
            && all_values[i] == original_values[i]
            && all_names[i] == original_names[i].map(std::string::ToString::to_string);

        if !matches {
            println!(
                "  MISMATCH at row {}: expected ({}, {:?}, {:?}), got ({}, {:?}, {:?})",
                i,
                original_id,
                original_values[i],
                original_names[i],
                all_ids[i],
                all_values[i],
                all_names[i]
            );
            all_match = false;
        }
    }

    if all_match {
        println!("✓ All {total_rows} rows match! Arrow roundtrip successful.");
    } else {
        println!("✗ Data mismatch detected!");
    }

    println!();
    Ok(())
}

/// Example 6: `ArrowReader` convenience methods
fn example_arrow_reader(connection: &Connection) -> Result<()> {
    println!("=== Example 6: ArrowReader Convenience Methods ===");

    // Create and populate a test table
    connection.execute_command("DROP TABLE IF EXISTS reader_test")?;
    connection.execute_command(
        "CREATE TABLE reader_test (
            id INTEGER NOT NULL,
            category TEXT,
            amount DOUBLE PRECISION
        )",
    )?;
    connection.execute_command(
        "INSERT INTO reader_test VALUES
            (1, 'Electronics', 299.99),
            (2, 'Books', 24.99),
            (3, 'Electronics', 149.99),
            (4, 'Clothing', 79.99),
            (5, 'Books', 34.99)",
    )?;
    println!("Created and populated 'reader_test' table with 5 rows");

    // Create ArrowReader
    let reader = ArrowReader::new(connection);

    // Method 1: Query to Arrow
    let data = reader.query_to_arrow("SELECT * FROM reader_test WHERE category = 'Electronics'")?;
    let count = count_rows_in_arrow(&data);
    println!(
        "query_to_arrow (Electronics filter): {} rows, {} bytes",
        count,
        data.len()
    );

    // Method 2: Table to Arrow
    let data = reader.table_to_arrow("reader_test")?;
    let count = count_rows_in_arrow(&data);
    println!("table_to_arrow: {} rows, {} bytes", count, data.len());

    // Method 3: Table columns to Arrow
    let data = reader.table_columns_to_arrow("reader_test", &["id", "amount"])?;
    let count = count_rows_in_arrow(&data);
    println!(
        "table_columns_to_arrow (id, amount): {} rows, {} bytes",
        count,
        data.len()
    );

    // Method 4: Table filtered to Arrow
    let data = reader.table_filtered_to_arrow("reader_test", "amount > 50")?;
    let count = count_rows_in_arrow(&data);
    println!(
        "table_filtered_to_arrow (amount > 50): {} rows, {} bytes",
        count,
        data.len()
    );

    // Also demonstrate Connection convenience methods
    let data = connection.execute_query_to_arrow("SELECT * FROM reader_test")?;
    println!("Connection.execute_query_to_arrow: {} bytes", data.len());

    let data = connection.export_table_to_arrow("reader_test")?;
    println!("Connection.export_table_to_arrow: {} bytes", data.len());

    println!();
    Ok(())
}

/// Example 7: Struct mapping from Arrow data
fn example_struct_mapping_from_arrow(connection: &Connection) -> Result<()> {
    println!("=== Example 7: Struct Mapping from Arrow Data ===");

    // Create and populate a table
    connection.execute_command("DROP TABLE IF EXISTS arrow_struct_test")?;
    connection.execute_command(
        "CREATE TABLE arrow_struct_test (
            id INTEGER NOT NULL,
            name TEXT NOT NULL,
            email TEXT,
            balance DOUBLE PRECISION NOT NULL
        )",
    )?;
    connection.execute_command(
        "INSERT INTO arrow_struct_test VALUES
            (1, 'Alice', 'alice@example.com', 1500.50),
            (2, 'Bob', 'bob@example.com', 2300.00),
            (3, 'Charlie', NULL, 890.25),
            (4, 'Diana', 'diana@example.com', 3100.00),
            (5, 'Eve', 'eve@example.com', 1750.75)",
    )?;
    println!("Created and populated 'arrow_struct_test' table with 5 rows");

    // Define a struct to represent our data
    #[derive(Debug, Clone)]
    struct User {
        id: i32,
        name: String,
        email: Option<String>,
        balance: f64,
    }

    impl User {
        /// Convert Arrow arrays into a User struct
        fn from_arrow_arrays(
            id_array: &Int32Array,
            name_array: &StringArray,
            email_array: &StringArray,
            balance_array: &Float64Array,
            index: usize,
        ) -> Self {
            Self {
                id: id_array.value(index),
                name: name_array.value(index).to_string(),
                email: if email_array.is_null(index) {
                    None
                } else {
                    Some(email_array.value(index).to_string())
                },
                balance: balance_array.value(index),
            }
        }
    }

    // Read query results as Arrow IPC stream
    let reader = ArrowReader::new(connection);
    let arrow_data = reader
        .query_to_arrow("SELECT id, name, email, balance FROM arrow_struct_test ORDER BY id")?;
    println!("Read {} bytes of Arrow IPC data", arrow_data.len());

    // Parse Arrow IPC stream and map to structs
    let cursor = std::io::Cursor::new(&arrow_data);
    let stream_reader =
        StreamReader::try_new(cursor, None).expect("Failed to create Arrow StreamReader");

    let mut users: Vec<User> = Vec::new();

    for batch_result in stream_reader {
        let batch = batch_result.expect("Failed to read batch");

        // Extract arrays from the batch
        let id_array = batch
            .column(0)
            .as_any()
            .downcast_ref::<Int32Array>()
            .expect("Expected Int32Array for id");
        let name_array = batch
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("Expected StringArray for name");
        let email_array = batch
            .column(2)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("Expected StringArray for email");
        let balance_array = batch
            .column(3)
            .as_any()
            .downcast_ref::<Float64Array>()
            .expect("Expected Float64Array for balance");

        // Map each row to a User struct
        for i in 0..batch.num_rows() {
            let user = User::from_arrow_arrays(id_array, name_array, email_array, balance_array, i);
            users.push(user);
        }
    }

    println!("\nMapped {} users from Arrow data:", users.len());
    for user in &users {
        println!(
            "  [ID: {}] {} ({}) - ${:.2}",
            user.id,
            user.name,
            user.email.as_deref().unwrap_or("No Email"),
            user.balance
        );
    }

    // Demonstrate using the structs
    let total_balance: f64 = users.iter().map(|u| u.balance).sum();
    let avg_balance = total_balance / users.len() as f64;
    println!("\nStatistics:");
    println!("  Total balance: ${total_balance:.2}");
    println!("  Average balance: ${avg_balance:.2}");
    println!(
        "  Users with email: {}",
        users.iter().filter(|u| u.email.is_some()).count()
    );

    println!();
    Ok(())
}

/// Helper to count rows in Arrow IPC data
fn count_rows_in_arrow(data: &[u8]) -> usize {
    let cursor = std::io::Cursor::new(data);
    match StreamReader::try_new(cursor, None) {
        Ok(reader) => reader
            .into_iter()
            .filter_map(std::result::Result::ok)
            .map(|batch| batch.num_rows())
            .sum(),
        Err(_) => 0,
    }
}

// =============================================================================
// Arrow Data Generation Helpers
// =============================================================================

/// Generates sample Arrow IPC data with 5 rows.
fn generate_sample_arrow_data() -> Vec<u8> {
    // Define schema matching the Hyper table
    let schema = Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("value", DataType::Float64, true),
        Field::new("name", DataType::Utf8, true),
    ]);

    // Create arrays
    let id_array = Int32Array::from(vec![1, 2, 3, 4, 5]);
    let value_array = Float64Array::from(vec![Some(1.5), Some(2.5), None, Some(4.5), Some(5.5)]);
    let name_array = StringArray::from(vec![
        Some("Alice"),
        Some("Bob"),
        Some("Charlie"),
        None,
        Some("Eve"),
    ]);

    // Create record batch
    let batch = RecordBatch::try_new(
        Arc::new(schema.clone()),
        vec![
            Arc::new(id_array),
            Arc::new(value_array),
            Arc::new(name_array),
        ],
    )
    .expect("Failed to create record batch");

    // Write to IPC stream
    write_arrow_ipc_stream(&schema, &[batch])
}

/// Generates Arrow IPC data with multiple record batches.
fn generate_multi_batch_arrow_data() -> Vec<u8> {
    let schema = Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("category", DataType::Utf8, true),
    ]);

    // Create multiple batches
    let batch1 = RecordBatch::try_new(
        Arc::new(schema.clone()),
        vec![
            Arc::new(Int32Array::from(vec![1, 2, 3])),
            Arc::new(StringArray::from(vec![Some("A"), Some("B"), Some("C")])),
        ],
    )
    .expect("Failed to create batch 1");

    let batch2 = RecordBatch::try_new(
        Arc::new(schema.clone()),
        vec![
            Arc::new(Int32Array::from(vec![4, 5, 6])),
            Arc::new(StringArray::from(vec![Some("D"), None, Some("F")])),
        ],
    )
    .expect("Failed to create batch 2");

    let batch3 = RecordBatch::try_new(
        Arc::new(schema.clone()),
        vec![
            Arc::new(Int32Array::from(vec![7, 8, 9, 10])),
            Arc::new(StringArray::from(vec![
                Some("G"),
                Some("H"),
                Some("I"),
                Some("J"),
            ])),
        ],
    )
    .expect("Failed to create batch 3");

    write_arrow_ipc_stream(&schema, &[batch1, batch2, batch3])
}

/// Generates Arrow IPC data for the `existing_table` schema.
fn generate_existing_table_arrow_data() -> Vec<u8> {
    let schema = Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("data", DataType::Utf8, true),
        Field::new("value", DataType::Float64, true),
    ]);

    let batch = RecordBatch::try_new(
        Arc::new(schema.clone()),
        vec![
            Arc::new(Int32Array::from(vec![100, 200, 300])),
            Arc::new(StringArray::from(vec![Some("data1"), Some("data2"), None])),
            Arc::new(Float64Array::from(vec![Some(10.0), None, Some(30.0)])),
        ],
    )
    .expect("Failed to create record batch");

    write_arrow_ipc_stream(&schema, &[batch])
}

/// Generates a random string up to `max_len` characters.
///
/// Uses a deterministic but varied approach based on the seed to generate
/// strings with varying lengths and content.
fn generate_random_string(seed: usize, max_len: usize) -> String {
    // Use seed to determine string length (1 to max_len)
    let len = (seed % max_len) + 1;

    // Generate characters using a simple pseudo-random approach
    let mut result = String::with_capacity(len);
    for i in 0..len {
        // Use a combination of seed and position to generate varied characters
        let char_code = (seed.wrapping_mul(31).wrapping_add(i)) % 62;
        let c = if char_code < 26 {
            // Lowercase letters
            (b'a' + char_code as u8) as char
        } else if char_code < 52 {
            // Uppercase letters
            (b'A' + (char_code - 26) as u8) as char
        } else {
            // Digits
            (b'0' + (char_code - 52) as u8) as char
        };
        result.push(c);
    }
    result
}

/// Writes record batches to Arrow IPC stream format.
fn write_arrow_ipc_stream(schema: &Schema, batches: &[RecordBatch]) -> Vec<u8> {
    let mut buffer = Vec::new();
    {
        let mut writer =
            StreamWriter::try_new(&mut buffer, schema).expect("Failed to create StreamWriter");
        for batch in batches {
            writer.write(batch).expect("Failed to write batch");
        }
        writer.finish().expect("Failed to finish stream");
    }
    buffer
}
