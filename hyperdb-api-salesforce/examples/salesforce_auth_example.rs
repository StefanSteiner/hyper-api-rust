// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Example: Salesforce Data Cloud OAuth Authentication
//!
//! This example demonstrates how to authenticate with Salesforce Data Cloud
//! using the JWT Bearer Token Flow and execute queries against Hyper.
//!
//! # Prerequisites
//!
//! 1. Create a Salesforce Connected App with:
//!    - OAuth enabled
//!    - "Use digital signatures" enabled with your public certificate
//!    - Required OAuth scopes (api, `cdp_query_api`)
//!
//! 2. Pre-authorize the connected app for your user:
//!    - In Salesforce Setup, go to Manage Connected Apps
//!    - Find your app and click "Edit Policies"
//!    - Set "Permitted Users" to "Admin approved users are pre-authorized"
//!    - Add your user's profile or permission set
//!
//! 3. Generate RSA key pair and upload public certificate to the Connected App
//!
//! # Environment Variables
//!
//! - `SF_LOGIN_URL`: Salesforce login URL (e.g., "<https://login.salesforce.com>")
//! - `SF_CLIENT_ID`: Connected App Consumer Key
//! - `SF_USERNAME`: Salesforce username (email)
//! - `SF_PRIVATE_KEY_PATH`: Path to RSA private key file (PEM format)
//! - `SF_DATASPACE`: (Optional) Data Cloud dataspace name
//!
//! # Running
//!
//! ```bash
//! export SF_LOGIN_URL="https://login.salesforce.com"
//! export SF_CLIENT_ID="your-connected-app-consumer-key"
//! export SF_USERNAME="user@example.com"
//! export SF_PRIVATE_KEY_PATH="/path/to/private_key.pem"
//!
//! cargo run -p hyperdb-api-salesforce --example salesforce_auth_example
//! ```

#![allow(clippy::cast_precision_loss, reason = "example timing diagnostics")]

use std::env;
use std::fs;

use hyperdb_api_salesforce::{
    AuthMode, DataCloudTokenProvider, SalesforceAuthConfig, SharedTokenProvider,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing for info output only (no debug)
    tracing_subscriber::fmt()
        .with_env_filter("hyperdb_api_salesforce=info,hyperdb_api_core=info")
        .init();

    println!("=== Salesforce Data Cloud OAuth Authentication Example ===\n");

    // Read configuration from environment variables
    let login_url =
        env::var("SF_LOGIN_URL").unwrap_or_else(|_| "https://login.salesforce.com".to_string());

    let Ok(client_id) = env::var("SF_CLIENT_ID") else {
        println!("Error: SF_CLIENT_ID environment variable is required.\n");
        println!("Please set the following environment variables:");
        println!("  export SF_CLIENT_ID=\"your-connected-app-consumer-key\"");
        println!("  export SF_USERNAME=\"user@example.com\"");
        println!("  export SF_PRIVATE_KEY_PATH=\"/path/to/private_key.pem\"");
        println!("  export SF_LOGIN_URL=\"https://login.salesforce.com\"  # optional");
        println!("  export SF_DATASPACE=\"default\"  # optional");
        std::process::exit(1);
    };

    let Ok(username) = env::var("SF_USERNAME") else {
        println!("Error: SF_USERNAME environment variable is required.");
        std::process::exit(1);
    };

    let Ok(private_key_path) = env::var("SF_PRIVATE_KEY_PATH") else {
        println!("Error: SF_PRIVATE_KEY_PATH environment variable is required.");
        std::process::exit(1);
    };

    let dataspace = env::var("SF_DATASPACE").ok();

    println!("Configuration:");
    println!("  Login URL:  {login_url}");
    println!("  Client ID:  {}...", &client_id[..20.min(client_id.len())]);
    println!("  Username:   {username}");
    println!("  Key Path:   {private_key_path}");
    println!(
        "  Dataspace:  {}",
        dataspace.as_deref().unwrap_or("(default)")
    );
    println!();

    // Load the private key
    let private_key_pem = fs::read_to_string(&private_key_path)
        .map_err(|e| format!("Failed to read private key file: {e}"))?;

    // Create authentication configuration
    let mut config = SalesforceAuthConfig::new(&login_url, &client_id)?
        .auth_mode(AuthMode::private_key(&username, &private_key_pem)?);

    if let Some(ref ds) = dataspace {
        config = config.dataspace(ds);
    }

    // Create the token provider
    let mut provider = DataCloudTokenProvider::new(config)?;

    println!("Authenticating with Salesforce...\n");

    // Get a Data Cloud token
    let token = provider.get_token().await?;

    println!("Authentication successful!");
    println!("  Token type:   {}", token.token_type());
    println!("  Tenant URL:   {}", token.tenant_url());
    println!("  Expires at:   {}", token.expires_at());
    println!("  Token valid:  {}", token.is_valid());

    // Try to extract tenant ID from the JWT
    match token.tenant_id() {
        Ok(tenant_id) => println!("  Tenant ID:    {tenant_id}"),
        Err(e) => println!("  Tenant ID:    (could not extract: {e})"),
    }

    // Get the lakehouse name for Hyper connection
    match token.lakehouse_name(dataspace.as_deref()) {
        Ok(name) => println!("  Lakehouse:    {name}"),
        Err(e) => println!("  Lakehouse:    (could not construct: {e})"),
    }

    println!();

    // Demonstrate token refresh
    println!("Testing token refresh...");
    let _refreshed = provider.refresh_token().await?;
    println!("Token refreshed successfully!\n");

    // ==========================================================================
    // RECOMMENDED: Use AuthenticatedGrpcClient for automatic token refresh
    // ==========================================================================
    //
    // AuthenticatedGrpcClient handles token expiration automatically:
    // 1. Proactive refresh: Refreshes token before it expires (2 min buffer)
    // 2. Reactive refresh: Retries on auth errors with fresh token
    //
    // This is essential for long-running queries (>15 min) in ASYNC/ADAPTIVE modes.

    {
        use hyperdb_api_core::client::grpc::AuthenticatedGrpcClient;

        println!("=== Connecting to Data Cloud via AuthenticatedGrpcClient ===\n");
        println!("This client automatically handles JWT token refresh for long queries.\n");

        // Create a shared token provider from the existing config
        let auth_config = SalesforceAuthConfig::new(&login_url, &client_id)?
            .auth_mode(AuthMode::private_key(&username, &private_key_pem)?);
        let auth_config = if let Some(ref ds) = dataspace {
            auth_config.dataspace(ds)
        } else {
            auth_config
        };

        let token_provider = SharedTokenProvider::new(auth_config)?;

        // Create the authenticated client - it handles all token management
        let mut client =
            AuthenticatedGrpcClient::connect(token_provider, dataspace.clone()).await?;

        println!("Connected successfully!");
        if let Some(token) = client.current_token() {
            println!("  Tenant URL:   {}", token.tenant_url());
            println!("  Token valid:  {}", token.is_valid());
            println!("  Expires at:   {}", token.expires_at());
        }
        println!();

        // Query to list all available tables in Data Cloud
        // Uses the built-in list_tables() API method instead of raw pg_catalog queries
        println!("=== Querying Available Tables ===\n");

        println!("Using client.list_tables_with_limit() API...");
        match client.list_tables_with_limit(Some(50)).await {
            Ok(mut tables) => {
                println!("Found {} tables\n", tables.len());

                // Fetch table display names for the public schema
                println!("Fetching table display names...");
                let table_labels = client.get_table_labels("public").await.unwrap_or_default();

                // Update tables with display names
                for table in &mut tables {
                    if let Some(display_name) = table_labels.get(&table.name) {
                        table.display_name = Some(display_name.clone());
                    }
                }
                println!("Found {} table labels\n", table_labels.len());

                println!("Available Tables:");
                println!("{:<30} {:<50} {:<15}", "Schema", "Table Name", "Type");
                println!("{}", "-".repeat(95));

                let mut first_table: Option<(String, String)> = None;
                for (idx, table) in tables.iter().enumerate() {
                    if idx < 20 {
                        let display = if let Some(ref dn) = table.display_name {
                            if dn == &table.name {
                                table.name.clone()
                            } else {
                                format!("{} ({})", dn, table.name)
                            }
                        } else {
                            table.name.clone()
                        };
                        println!(
                            "{:<30} {:<50} {:<15}",
                            table.schema, display, table.table_type
                        );
                    }
                    if first_table.is_none() {
                        first_table = Some((table.schema.clone(), table.name.clone()));
                    }
                }

                if tables.len() > 20 {
                    println!(
                        "\n... and {} more tables (showing first 20)",
                        tables.len() - 20
                    );
                }
                println!("\nTotal tables found: {}", tables.len());

                // Count rows in all tables to find the largest one
                println!("\n=== Counting Rows in All Tables ===\n");
                let mut table_sizes: Vec<(String, String, i64)> = Vec::new();

                use arrow::ipc::reader::StreamReader;
                use std::io::Cursor;

                for table in &tables {
                    // Count rows in this table
                    let count_query = format!(
                        "SELECT COUNT(*) as row_count FROM {}.{}",
                        table.schema, table.name
                    );
                    if let Ok(result) = client.execute_query(&count_query).await {
                        if let Ok(count_reader) =
                            StreamReader::try_new(Cursor::new(result.arrow_data()), None)
                        {
                            for count_batch in count_reader.flatten() {
                                if let Some(count_arr) = count_batch
                                    .column(0)
                                    .as_any()
                                    .downcast_ref::<arrow::array::Int64Array>()
                                {
                                    let row_count = count_arr.value(0);
                                    let display = if let Some(ref dn) = table.display_name {
                                        if dn == &table.name {
                                            format!("{}.{}", table.schema, table.name)
                                        } else {
                                            format!("{} ({}.{})", dn, table.schema, table.name)
                                        }
                                    } else {
                                        format!("{}.{}", table.schema, table.name)
                                    };
                                    println!("  {display}: {row_count} rows");
                                    table_sizes.push((
                                        table.schema.clone(),
                                        table.name.clone(),
                                        row_count,
                                    ));
                                }
                            }
                        }
                    }
                }

                // Sort by row count descending and pick the largest
                table_sizes.sort_by_key(|b| std::cmp::Reverse(b.2));
                let (schema, table, row_count) = table_sizes
                    .first()
                    .map(|(s, t, c)| (s.clone(), t.clone(), *c))
                    .or_else(|| first_table.map(|(s, t)| (s, t, 0)))
                    .unwrap();

                println!("\n✓ Selected largest table: {schema}.{table} ({row_count} rows)\n");

                // Query selected table for sample data
                {
                    println!("\n=== Querying Sample Data from First Table ===\n");
                    println!("Table: {schema}.{table}");

                    // Fetch column labels using the built-in API method
                    println!("Fetching column labels from metadata...");
                    let column_labels = client
                        .get_column_labels(&schema, &table)
                        .await
                        .unwrap_or_default();
                    if column_labels.is_empty() {
                        println!("No column labels found in metadata, using API names\n");
                    } else {
                        println!("Found {} column labels\n", column_labels.len());
                    }

                    let sample_query = format!("SELECT * FROM {schema}.{table} LIMIT 10");

                    println!("Executing query: {sample_query}\n");
                    match client.execute_query(&sample_query).await {
                        Ok(result) => {
                            let arrow_data = result.arrow_data();
                            println!(
                                "Query successful! Received {} bytes of Arrow data\n",
                                arrow_data.len()
                            );

                            match StreamReader::try_new(Cursor::new(arrow_data), None) {
                                Ok(reader) => {
                                    use arrow::util::display::array_value_to_string;

                                    for batch_result in reader {
                                        match batch_result {
                                            Ok(batch) => {
                                                let batch_schema = batch.schema();

                                                // Get column names and their display labels
                                                let col_info: Vec<(String, String)> = batch_schema
                                                    .fields()
                                                    .iter()
                                                    .map(|f| {
                                                        let api_name = f.name().clone();
                                                        let display_name = column_labels
                                                            .get(&api_name)
                                                            .cloned()
                                                            .unwrap_or_else(|| api_name.clone());
                                                        (api_name, display_name)
                                                    })
                                                    .collect();

                                                println!("Columns ({}):", col_info.len());
                                                for (idx, (api_name, display_name)) in
                                                    col_info.iter().enumerate()
                                                {
                                                    if api_name == display_name {
                                                        print!("  [{idx}] {api_name}");
                                                    } else {
                                                        print!(
                                                            "  [{idx}] {display_name} ({api_name})"
                                                        );
                                                    }
                                                    if (idx + 1) % 3 == 0 {
                                                        println!();
                                                    } else {
                                                        print!("  ");
                                                    }
                                                }
                                                println!("\n\nRows: {}\n", batch.num_rows());

                                                // Display first 3 rows using Arrow's built-in formatter
                                                for row_idx in 0..batch.num_rows().min(3) {
                                                    println!("Row {}:", row_idx + 1);
                                                    #[expect(
                                                        clippy::needless_range_loop,
                                                        reason = "loop body indexes multiple parallel slices (batch columns and col_info); enumerated iterator would obscure intent"
                                                    )]
                                                    for col_idx in 0..batch.num_columns() {
                                                        let col = batch.column(col_idx);
                                                        let value =
                                                            array_value_to_string(col, row_idx)
                                                                .unwrap_or_else(|_| {
                                                                    "?".to_string()
                                                                });
                                                        let (api_name, display_name) =
                                                            &col_info[col_idx];
                                                        if api_name == display_name {
                                                            println!("  {api_name}: {value}");
                                                        } else {
                                                            println!(
                                                                "  {display_name} ({api_name}): {value}"
                                                            );
                                                        }
                                                    }
                                                    println!();
                                                }
                                            }
                                            Err(e) => println!("Error reading batch: {e}"),
                                        }
                                    }
                                }
                                Err(e) => println!("Failed to parse Arrow data: {e}"),
                            }
                        }
                        Err(e) => println!("Query failed: {e}"),
                    }

                    // Now query the full table for performance metrics
                    println!("\n=== Performance Test: Querying Full Table ===\n");
                    let full_query = format!("SELECT * FROM {schema}.{table}");
                    println!("Executing query: {full_query}\n");

                    let start = std::time::Instant::now();
                    match client.execute_query(&full_query).await {
                        Ok(result) => {
                            let elapsed = start.elapsed();
                            let arrow_data = result.arrow_data();
                            let total_bytes = arrow_data.len();

                            // Count total rows
                            let mut total_rows = 0;
                            match StreamReader::try_new(Cursor::new(arrow_data), None) {
                                Ok(reader) => {
                                    for batch in reader.flatten() {
                                        total_rows += batch.num_rows();
                                    }
                                }
                                Err(e) => println!("Failed to parse Arrow data: {e}"),
                            }

                            let elapsed_secs = elapsed.as_secs_f64();
                            let rows_per_sec = total_rows as f64 / elapsed_secs;
                            let bytes_per_sec = total_bytes as f64 / elapsed_secs;
                            let mb_per_sec = bytes_per_sec / (1024.0 * 1024.0);

                            println!("Performance Metrics:");
                            println!("  Total rows:        {total_rows}");
                            println!(
                                "  Total bytes:       {} ({:.2} MB)",
                                total_bytes,
                                total_bytes as f64 / (1024.0 * 1024.0)
                            );
                            println!("  Query time:        {elapsed_secs:.3} seconds");
                            println!("  Rows/second:       {rows_per_sec:.2}");
                            println!(
                                "  Bytes/second:      {bytes_per_sec:.2} ({mb_per_sec:.2} MB/s)"
                            );
                        }
                        Err(e) => println!("Full table query failed: {e}"),
                    }
                }
            }
            Err(e) => {
                println!("Failed to list tables: {e}");
            }
        }
    }

    println!("\n=== Example Complete ===");

    Ok(())
}
