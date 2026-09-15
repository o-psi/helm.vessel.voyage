import { loadConfig } from './config.js';
import { createGateway } from './gateway.js';
try {
  const config = await loadConfig();
  const gateway = createGateway(config);
  gateway.server.on('error', () => { console.error('gateway listener failed'); process.exitCode = 1; void gateway.close(); });
  gateway.server.listen(config.port, config.host, () => console.log('gateway listening'));
  for (const signal of ['SIGINT', 'SIGTERM']) process.once(signal, () => { void gateway.close(); });
} catch { console.error('gateway configuration refused'); process.exitCode = 1; }
