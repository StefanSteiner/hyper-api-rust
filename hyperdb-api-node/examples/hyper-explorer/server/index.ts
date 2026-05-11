// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

import express from 'express';
import cors from 'cors';
import { createRequire } from 'module';
import { registerRoutes, ConnectionPool } from './routes.js';

const require = createRequire(import.meta.url);
const { HyperProcess } = require('hyperdb-api-node');

const app = express();
app.use(cors());
app.use(express.json());

// Start HyperProcess once
const hyper = new HyperProcess();
console.log(`[server] HyperProcess started at ${hyper.endpoint}`);
console.log(`[server] Hyper log path: ${hyper.logPath ?? '(unknown)'}`);

// Make hyper and connection pool available to routes
app.locals.hyper = hyper;
app.locals.hyperLogPath = hyper.logPath ?? null;
app.locals.pool = new ConnectionPool(hyper);

registerRoutes(app);

const PORT = process.env.PORT || 3000;
app.listen(PORT, () => {
  console.log(`[server] API listening on http://localhost:${PORT}`);
});

// Graceful shutdown
const shutdown = async () => {
  console.log('\n[server] Shutting down...');
  await app.locals.pool.closeAll();
  hyper.close();
  process.exit(0);
};
process.on('SIGINT', shutdown);
process.on('SIGTERM', shutdown);
