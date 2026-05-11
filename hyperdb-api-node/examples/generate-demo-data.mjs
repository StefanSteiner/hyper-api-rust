/**
 * Generates a demo .hyper database with ~10M rows across 3 tables,
 * showcasing varied data distributions and all key Hyper datatypes.
 *
 * Uses pure SQL (INSERT INTO ... SELECT + generate_series) for maximum speed.
 * Hyper's built-in random() drives all distributions.
 *
 * Tables:
 *   sensor_readings  (~5M rows) — IoT time-series: normal, uniform, bimodal distributions
 *   transactions      (~3M rows) — Financial: log-normal amounts, categorical, skewed booleans
 *   products          (~2M rows) — E-commerce: exponential prices, Zipf brands, varied nullability
 *
 * Usage:
 *   HYPERD_PATH=/path/to/hyperd node examples/generate-demo-data.mjs [output.hyper]
 */

import { createRequire } from 'module';
import { join, dirname } from 'path';
import { fileURLToPath } from 'url';
import { mkdirSync, existsSync, unlinkSync, statSync } from 'fs';

const require = createRequire(import.meta.url);
const { HyperProcess, Connection, CreateMode } = require('../index.js');

const __dirname = dirname(fileURLToPath(import.meta.url));
const OUTPUT_DIR = join(__dirname, '..', 'test_results');
mkdirSync(OUTPUT_DIR, { recursive: true });

const outputPath = process.argv[2] || join(OUTPUT_DIR, 'demo_data.hyper');

// Helper: run SQL, print timing
async function exec(conn, label, sql) {
  const start = Date.now();
  await conn.executeCommand(sql);
  const ms = Date.now() - start;
  console.log(`   ${label} (${(ms / 1000).toFixed(1)}s)`);
}

async function main() {
  console.log('=== Demo Data Generator ===\n');
  const t0 = Date.now();

  if (existsSync(outputPath)) {
    unlinkSync(outputPath);
    console.log(`Removed existing ${outputPath}`);
  }

  const hyper = new HyperProcess();
  console.log(`HyperProcess: ${hyper.endpoint}`);

  const conn = await Connection.connect(hyper.endpoint, outputPath, CreateMode.CreateIfNotExists);
  console.log(`Database: ${outputPath}\n`);

  // ==========================================================================
  // Table 1: sensor_readings (5M rows)
  // Distributions: normal, uniform, bimodal, skewed bool, power-law text
  // Types: INT, SMALLINT, TIMESTAMP, DOUBLE, BOOL, TEXT
  // ==========================================================================
  console.log('Creating sensor_readings (5M rows)...');

  await exec(conn, 'CREATE TABLE', `
    CREATE TABLE sensor_readings (
      id              INT          NOT NULL,
      sensor_id       SMALLINT     NOT NULL,
      reading_time    TIMESTAMP    NOT NULL,
      temperature     DOUBLE PRECISION NOT NULL,
      humidity        DOUBLE PRECISION,
      pressure        DOUBLE PRECISION NOT NULL,
      battery_pct     DOUBLE PRECISION,
      is_anomaly      BOOLEAN      NOT NULL,
      status          TEXT         NOT NULL,
      error_code      SMALLINT
    )
  `);

  // Box-Muller normal via SQL: sqrt(-2*ln(r1)) * cos(2*pi*r2) * stddev + mean
  // Bimodal: CASE on random() to pick one of two normals
  await exec(conn, 'INSERT 5M rows', `
    INSERT INTO sensor_readings
    SELECT
      i AS id,
      CAST(1 + FLOOR(random() * 500) AS SMALLINT) AS sensor_id,
      TIMESTAMP '2023-01-01 00:00:00'
        + CAST(FLOOR(random() * 94608000) AS INT) * INTERVAL '1 second' AS reading_time,

      -- temperature: normal(22, 5)
      ROUND(CAST(
        22.0 + SQRT(-2.0 * LN(GREATEST(random(), 1e-10))) * COS(2.0 * 3.141592653589793 * random()) * 5.0
      AS DOUBLE PRECISION), 2) AS temperature,

      -- humidity: uniform(20, 95), 2% null
      CASE WHEN random() < 0.02 THEN NULL
           ELSE ROUND(CAST(20.0 + random() * 75.0 AS DOUBLE PRECISION), 1)
      END AS humidity,

      -- pressure: bimodal — 70% normal(1013,5), 30% normal(985,8)
      ROUND(CAST(
        CASE WHEN random() < 0.7
          THEN 1013.0 + SQRT(-2.0 * LN(GREATEST(random(), 1e-10))) * COS(2.0 * 3.141592653589793 * random()) * 5.0
          ELSE  985.0 + SQRT(-2.0 * LN(GREATEST(random(), 1e-10))) * COS(2.0 * 3.141592653589793 * random()) * 8.0
        END
      AS DOUBLE PRECISION), 1) AS pressure,

      -- battery_pct: normal(75,20) clamped [0,100], 5% null
      CASE WHEN random() < 0.05 THEN NULL
           ELSE ROUND(CAST(GREATEST(0.0, LEAST(100.0,
             75.0 + SQRT(-2.0 * LN(GREATEST(random(), 1e-10))) * COS(2.0 * 3.141592653589793 * random()) * 20.0
           )) AS DOUBLE PRECISION), 1)
      END AS battery_pct,

      -- is_anomaly: 5% true
      random() < 0.05 AS is_anomaly,

      -- status: power-law categorical
      CASE CAST(FLOOR(
        CASE
          WHEN random() < 0.70 THEN 0
          WHEN random() < 0.82 THEN 1
          WHEN random() < 0.90 THEN 2
          WHEN random() < 0.95 THEN 3
          WHEN random() < 0.98 THEN 4
          ELSE 5
        END
      ) AS INT)
        WHEN 0 THEN 'ok'
        WHEN 1 THEN 'warning'
        WHEN 2 THEN 'calibrating'
        WHEN 3 THEN 'error'
        WHEN 4 THEN 'offline'
        ELSE 'maintenance'
      END AS status,

      -- error_code: 92% null
      CASE WHEN random() < 0.92 THEN NULL
           ELSE CAST((ARRAY[100,200,201,300,301,302,400,401,500])[CAST(1 + FLOOR(random() * 9) AS INT)] AS SMALLINT)
      END AS error_code

    FROM generate_series(1, 5000000) AS s(i)
  `);

  // ==========================================================================
  // Table 2: transactions (3M rows)
  // Distributions: log-normal amounts, weighted categoricals, skewed booleans
  // Types: BIGINT, INT, DOUBLE, TEXT, DATE, BOOL, SMALLINT
  // ==========================================================================
  console.log('\nCreating transactions (3M rows)...');

  await exec(conn, 'CREATE TABLE', `
    CREATE TABLE transactions (
      id                BIGINT       NOT NULL,
      customer_id       INT          NOT NULL,
      amount            DOUBLE PRECISION NOT NULL,
      tax               DOUBLE PRECISION,
      category          TEXT         NOT NULL,
      payment_method    TEXT         NOT NULL,
      transaction_date  DATE         NOT NULL,
      is_fraud          BOOLEAN      NOT NULL,
      is_refunded       BOOLEAN      NOT NULL,
      rating            SMALLINT,
      notes             TEXT
    )
  `);

  // Log-normal via SQL: EXP(normal(mu, sigma))
  await exec(conn, 'INSERT 3M rows', `
    INSERT INTO transactions
    SELECT
      CAST(i AS BIGINT) AS id,
      CAST(1 + FLOOR(random() * 200000) AS INT) AS customer_id,

      -- amount: log-normal, median ~$33, long right tail, capped at 50000
      LEAST(50000.0, ROUND(CAST(
        EXP(3.5 + SQRT(-2.0 * LN(GREATEST(random(), 1e-10))) * COS(2.0 * 3.141592653589793 * random()) * 1.2)
      AS DOUBLE PRECISION), 2)) AS amount,

      -- tax: ~3% null, otherwise 5-12% of amount
      CASE WHEN random() < 0.03 THEN NULL
           ELSE ROUND(CAST(
             LEAST(50000.0, EXP(3.5 + SQRT(-2.0 * LN(GREATEST(random(), 1e-10))) * COS(2.0 * 3.141592653589793 * random()) * 1.2))
             * (0.05 + random() * 0.07)
           AS DOUBLE PRECISION), 2)
      END AS tax,

      -- category: 15 categories with different weights
      CASE CAST(FLOOR(
        CASE
          WHEN random() < 0.155 THEN 0    -- Groceries (20-ish)
          WHEN random() < 0.30  THEN 1    -- Electronics
          WHEN random() < 0.44  THEN 2    -- Restaurant
          WHEN random() < 0.56  THEN 3    -- Clothing
          WHEN random() < 0.66  THEN 4    -- Entertainment
          WHEN random() < 0.74  THEN 5    -- Travel
          WHEN random() < 0.81  THEN 6    -- Subscription
          WHEN random() < 0.87  THEN 7    -- Health
          WHEN random() < 0.92  THEN 8    -- Home & Garden
          WHEN random() < 0.95  THEN 9    -- Utilities
          WHEN random() < 0.97  THEN 10   -- Automotive
          WHEN random() < 0.98  THEN 11   -- Sports
          WHEN random() < 0.99  THEN 12   -- Insurance
          WHEN random() < 0.995 THEN 13   -- Books
          ELSE 14                          -- Education
        END
      ) AS INT)
        WHEN 0  THEN 'Groceries'
        WHEN 1  THEN 'Electronics'
        WHEN 2  THEN 'Restaurant'
        WHEN 3  THEN 'Clothing'
        WHEN 4  THEN 'Entertainment'
        WHEN 5  THEN 'Travel'
        WHEN 6  THEN 'Subscription'
        WHEN 7  THEN 'Health'
        WHEN 8  THEN 'Home & Garden'
        WHEN 9  THEN 'Utilities'
        WHEN 10 THEN 'Automotive'
        WHEN 11 THEN 'Sports'
        WHEN 12 THEN 'Insurance'
        WHEN 13 THEN 'Books'
        ELSE 'Education'
      END AS category,

      -- payment_method: weighted
      CASE
        WHEN random() < 0.35 THEN 'Credit Card'
        WHEN random() < 0.60 THEN 'Debit Card'
        WHEN random() < 0.78 THEN 'PayPal'
        WHEN random() < 0.88 THEN 'Bank Transfer'
        WHEN random() < 0.96 THEN 'Cash'
        ELSE 'Crypto'
      END AS payment_method,

      -- transaction_date: 2020-2025
      DATE '2020-01-01' + CAST(FLOOR(random() * 2190) AS INT) AS transaction_date,

      random() < 0.005 AS is_fraud,
      random() < 0.03  AS is_refunded,

      -- rating: 1-5 skewed toward 4-5, 40% null
      CASE WHEN random() < 0.40 THEN NULL
           ELSE CAST(
             CASE
               WHEN random() < 0.02 THEN 1
               WHEN random() < 0.07 THEN 2
               WHEN random() < 0.22 THEN 3
               WHEN random() < 0.57 THEN 4
               ELSE 5
             END
           AS SMALLINT)
      END AS rating,

      -- notes: 85% null, otherwise 1-3 phrases
      CASE WHEN random() < 0.85 THEN NULL
           ELSE (ARRAY[
             'Delivered on time', 'Item was damaged', 'Great quality',
             'Not as described', 'Fast shipping', 'Wrong item received',
             'Perfect condition', 'Good value for money', 'Will buy again',
             'Needs improvement', 'Excellent service', 'Late delivery',
             'Refund requested', 'Gift purchase', 'Bulk order'
           ])[CAST(1 + FLOOR(random() * 15) AS INT)]
           || '. '
           || (ARRAY[
             'Delivered on time', 'Item was damaged', 'Great quality',
             'Not as described', 'Fast shipping', 'Wrong item received',
             'Perfect condition', 'Good value for money', 'Will buy again',
             'Needs improvement', 'Excellent service', 'Late delivery',
             'Refund requested', 'Gift purchase', 'Bulk order'
           ])[CAST(1 + FLOOR(random() * 15) AS INT)]
      END AS notes

    FROM generate_series(1, 3000000) AS s(i)
  `);

  // ==========================================================================
  // Table 3: products (2M rows)
  // Distributions: exponential prices, Zipf brands, correlated cost, varied nullability
  // Types: INT, TEXT, DOUBLE, BOOL, TIMESTAMP
  // ==========================================================================
  console.log('\nCreating products (2M rows)...');

  await exec(conn, 'CREATE TABLE', `
    CREATE TABLE products (
      id              INT          NOT NULL,
      name            TEXT         NOT NULL,
      price           DOUBLE PRECISION NOT NULL,
      cost            DOUBLE PRECISION NOT NULL,
      weight_kg       DOUBLE PRECISION,
      category        TEXT         NOT NULL,
      brand           TEXT         NOT NULL,
      in_stock        BOOLEAN      NOT NULL,
      stock_quantity  INT,
      rating_avg      DOUBLE PRECISION,
      review_count    INT,
      created_at      TIMESTAMP    NOT NULL,
      discontinued    BOOLEAN      NOT NULL
    )
  `);

  // Exponential via SQL: -ln(1-r)/lambda
  await exec(conn, 'INSERT 2M rows', `
    INSERT INTO products
    SELECT
      i AS id,

      -- name: brand + adjective + noun + number
      (ARRAY['Apex','Zenith','Pulse','Nova','Orbit','Summit','Ridge','Flux',
             'Crest','Prime','Echo','Forge','Drift','Aura','Bolt','Catalyst',
             'Vertex','Nimbus','Quasar','Titan'])[CAST(1 + FLOOR(random() * 20) AS INT)]
      || ' '
      || (ARRAY['Premium','Ultra','Pro','Classic','Essential','Advanced','Compact',
                'Deluxe','Elite','Eco','Smart','Turbo','Slim','Heavy-Duty','Portable'])[CAST(1 + FLOOR(random() * 15) AS INT)]
      || ' '
      || (ARRAY['Widget','Gadget','Device','Kit','Set','Pack','Unit','Module',
                'System','Tool','Sensor','Controller','Adapter','Hub','Station'])[CAST(1 + FLOOR(random() * 15) AS INT)]
      || ' '
      || CAST(CAST(100 + FLOOR(random() * 9899) AS INT) AS TEXT)
      AS name,

      -- price: exponential(0.015) + 5, capped at 5000
      ROUND(CAST(LEAST(5000.0,
        5.0 + (-LN(GREATEST(1.0 - random(), 1e-10)) / 0.015)
      ) AS DOUBLE PRECISION), 2) AS price,

      -- cost: 30-70% of price
      ROUND(CAST(
        LEAST(5000.0, 5.0 + (-LN(GREATEST(1.0 - random(), 1e-10)) / 0.015))
        * (0.3 + random() * 0.4)
      AS DOUBLE PRECISION), 2) AS cost,

      -- weight_kg: normal(2.5, 1.5), min 0.01, 8% null
      CASE WHEN random() < 0.08 THEN NULL
           ELSE ROUND(CAST(GREATEST(0.01,
             2.5 + SQRT(-2.0 * LN(GREATEST(random(), 1e-10))) * COS(2.0 * 3.141592653589793 * random()) * 1.5
           ) AS DOUBLE PRECISION), 2)
      END AS weight_kg,

      -- category: 15 uniform categories
      (ARRAY['Electronics','Clothing','Home & Kitchen','Sports','Books',
             'Beauty','Toys','Automotive','Garden','Office',
             'Food & Beverage','Health','Pet Supplies','Tools','Jewelry'])[CAST(1 + FLOOR(random() * 15) AS INT)]
      AS category,

      -- brand: 50 brands, Zipf-like via layered random
      (ARRAY['Apex','Zenith','Pulse','Nova','Orbit','Summit','Ridge','Flux',
             'Crest','Prime','Echo','Forge','Drift','Aura','Bolt','Catalyst',
             'Vertex','Nimbus','Quasar','Titan','Helix','Lumen','Cipher','Vortex',
             'Atlas','Phoenix','Spark','Wave','Core','Prism','Nexus','Onyx',
             'Blaze','Frost','Ember','Storm','Terra','Zephyr','Cobalt','Iron',
             'Jade','Maple','Silk','Steel','Birch','Cedar','Coral','Dusk','Fern','Sage']
      )[CAST(1 + FLOOR(POWER(random(), 1.8) * 50) AS INT)]
      AS brand,

      random() < 0.80 AS in_stock,

      -- stock_quantity: exponential, 10% null
      CASE WHEN random() < 0.10 THEN NULL
           ELSE CAST(FLOOR(-LN(GREATEST(1.0 - random(), 1e-10)) / 0.02) AS INT)
      END AS stock_quantity,

      -- rating_avg: normal(3.8, 0.7) clipped [1,5], 15% null
      CASE WHEN random() < 0.15 THEN NULL
           ELSE ROUND(CAST(GREATEST(1.0, LEAST(5.0,
             3.8 + SQRT(-2.0 * LN(GREATEST(random(), 1e-10))) * COS(2.0 * 3.141592653589793 * random()) * 0.7
           )) AS DOUBLE PRECISION), 1)
      END AS rating_avg,

      -- review_count: exponential, 12% null
      CASE WHEN random() < 0.12 THEN NULL
           ELSE CAST(FLOOR(-LN(GREATEST(1.0 - random(), 1e-10)) / 0.01) AS INT)
      END AS review_count,

      -- created_at: 2018-2025
      TIMESTAMP '2018-01-01 00:00:00'
        + CAST(FLOOR(random() * 252288000) AS INT) * INTERVAL '1 second' AS created_at,

      random() < 0.08 AS discontinued

    FROM generate_series(1, 2000000) AS s(i)
  `);

  // ==========================================================================
  // Summary
  // ==========================================================================
  const elapsed = (Date.now() - t0) / 1000;
  const fileSizeMB = statSync(outputPath).size / (1024 * 1024);

  console.log('\n--- Verification ---');
  const tables = ['sensor_readings', 'transactions', 'products'];
  let total = 0;
  for (const name of tables) {
    const rows = await conn.executeQuery(`SELECT COUNT(*) FROM ${name}`);
    const count = Number(rows[0].getInt64(0));
    total += count;
    console.log(`   ${name}: ${count.toLocaleString()} rows`);
  }
  const rowsPerSec = total / elapsed;
  console.log(`\n   File size : ${fileSizeMB.toFixed(1)} MB`);
  console.log(`   Time      : ${elapsed.toFixed(1)}s`);
  console.log(`   Throughput: ${(rowsPerSec / 1e6).toFixed(2)}M rows/s (${(fileSizeMB / elapsed).toFixed(1)} MB/s on disk)`);
  console.log(`\n=== Complete: ${total.toLocaleString()} total rows in ${outputPath} ===`);

  await conn.close();
  hyper.close();
}

main().catch((err) => {
  console.error('Error:', err);
  process.exit(1);
});
