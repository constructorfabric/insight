// Minimal JSON-line logger. One record per line so k8s log collectors
// (fluentbit, loki) can parse without extra config.
//
// Format: {"ts":"2026-05-22T...","level":"info","msg":"...","field":"..."}
//
// No deps. ~15 lines. If we ever need log levels filtering / sampling /
// correlation IDs / etc — swap for pino. For now this is enough.

function emit(level, msg, fields) {
  const record = {
    ts: new Date().toISOString(),
    level,
    msg,
    ...fields,
  };
  // stdout is line-buffered in non-TTY mode (k8s captures stdout)
  process.stdout.write(JSON.stringify(record) + '\n');
}

export const log = {
  info: (msg, fields = {}) => emit('info', msg, fields),
  warn: (msg, fields = {}) => emit('warn', msg, fields),
  error: (msg, fields = {}) => emit('error', msg, fields),
};
