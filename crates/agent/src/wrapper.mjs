// The guest half of kai-agent-session/1. All executable files are read-only.
// This module imports only the pinned SDK entry point and Node builtins.
import { query } from './sdk.mjs';
import net from 'node:net';
import { once } from 'node:events';

const FORMAT = 'kai-agent-session/1';
const MAX_FRAME = 2 * 1024 * 1024;
const MAX_PROMPT = 64 * 1024;
const socket = net.createConnection({ host: '127.0.0.1', port: 8788 });
let sequence = 0, queryHandle, active, started = false, closing = false;
let inputs = [], wake, pending = Buffer.alloc(0), lastTurn = 0;

// Diagnostics never echo a command, environment value or an SDK exception.
function fail(code) {
  if (closing) return;
  closing = true;
  queryHandle?.close();
  const line = JSON.stringify({ format: FORMAT, seq: ++sequence, kind: 'error', payload: { code } }) + '\n';
  socket.end(line, () => process.exit(1));
  setTimeout(() => process.exit(1), 1000).unref();
}
async function emit(kind, turn, session_id, payload) {
  const line = JSON.stringify({ format: FORMAT, seq: ++sequence, kind, turn, session_id, payload }) + '\n';
  if (Buffer.byteLength(line) > MAX_FRAME || socket.writableLength > MAX_FRAME) {
    fail('event-limit'); return;
  }
  if (!socket.write(line)) await once(socket, 'drain');
}
function safe(value) {
  if (Array.isArray(value)) return value.map(safe);
  if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value)
    .filter(([key]) => !['total_cost_usd', 'cost_usd', 'costUSD', 'totalCostUSD'].includes(key)).map(([key, item]) => [key, safe(item)]));
  // A tool may echo its environment. Do not send the route credential back.
  if (typeof value === 'string' && process.env.ANTHROPIC_API_KEY) return value.split(process.env.ANTHROPIC_API_KEY).join('[redacted]');
  return value;
}
async function* messages() {
  while (!closing) {
    if (!inputs.length) await new Promise(resolve => { wake = resolve; });
    while (inputs.length) yield inputs.shift();
  }
}
async function readEvents() {
  try {
    for await (const message of queryHandle) {
      if (closing) break;
      if (!active) { if (message.type === 'system') continue; fail('event-without-turn'); break; }
      const turn = active;
      if (message.type === 'result') {
        if (turn.interrupt) await turn.interrupt;
        active = undefined;
        // A turn the SDK finished successfully is an answer, even when an
        // interrupt was requested while it finished.
        const succeeded = message.subtype === 'success' && message.is_error === false;
        await emit('result', turn.id, message.session_id,
          safe({ ...message, interrupted: !!turn.interrupt && !succeeded }));
      } else if (['system', 'assistant', 'user', 'stream_event'].includes(message.type)) {
        // `system` carries the turn's `init`, whose `tools` are those Claude
        // actually offers the model, which may differ from the allowlist.
        await emit('event', turn.id, message.session_id, safe(message));
      }
    }
    if (!closing) fail('sdk-ended');
  } catch { if (!closing) fail('sdk-failed'); }
}
function exact(message, keys) {
  return message && typeof message === 'object' && !Array.isArray(message)
    && Object.keys(message).every(key => keys.includes(key));
}
async function command(m) {
  if (closing || m.format !== FORMAT) throw new Error('protocol');
  if (m.op === 'start') {
    const plugins = m.plugins ?? [];
    if (started || !exact(m, ['format', 'op', 'model', 'tools', 'plugins', 'max_turns']) || typeof m.model !== 'string'
      || !/^[A-Za-z0-9][A-Za-z0-9._:/-]{0,199}$/.test(m.model) || !Array.isArray(m.tools)
      || m.tools.some(tool => !['Read', 'Edit', 'Write', 'Bash', 'Glob', 'Grep'].includes(tool))
      || !Array.isArray(plugins) || plugins.length > 8 || new Set(plugins).size !== plugins.length
      || plugins.some(path => typeof path !== 'string' || !/^\/opt\/kai\/addons\/[a-z0-9][a-z0-9-]{0,63}$/.test(path))
      // The SDK takes 0 as no limit at all: a limit is at least 1.
      || (m.max_turns !== undefined && (!Number.isSafeInteger(m.max_turns) || m.max_turns < 1 || m.max_turns > 10000))) throw new Error('start');
    started = true;
    // Not bare: under --bare, Claude offers only Bash, Read and Edit. No
    // setting source either, so nothing of the tree configures Claude: its
    // CLAUDE.md, AGENTS.md, settings and hooks stay out. Empty, not omitted.
    // An add-on comes in as a local plugin from its read-only layer, the
    // one way a skill loads without a setting source. Claude replays each
    // user message, so the host sees which slash command it took.
    queryHandle = query({ prompt: messages(), options: {
      pathToClaudeCodeExecutable: '/opt/kai/agents/session/claude',
      cwd: '/job/tree', model: m.model, tools: m.tools, allowedTools: m.tools,
      includePartialMessages: true, settingSources: [],
      plugins: plugins.map(path => ({ type: 'local', path })),
      ...(m.max_turns === undefined ? {} : { maxTurns: m.max_turns }),
      extraArgs: { 'replay-user-messages': null },
      canUseTool: async () => ({ behavior: 'deny', message: 'Tool interaction is not enabled for this session.' }),
      env: { ...process.env }, stderr: () => {},
    } });
    await emit('ready', undefined, undefined, { sdk: '0.3.323', claude: '2.1.323', node: process.versions.node });
    void readEvents(); return;
  }
  if (!started) throw new Error('not-started');
  if (m.op === 'send') {
    if (!exact(m, ['format', 'op', 'turn', 'text']) || active || !Number.isSafeInteger(m.turn)
      || m.turn !== lastTurn + 1 || typeof m.text !== 'string' || Buffer.byteLength(m.text) > MAX_PROMPT) throw new Error('send');
    lastTurn = m.turn; active = { id: m.turn };
    inputs.push({ type: 'user', message: { role: 'user', content: m.text }, parent_tool_use_id: null });
    wake?.(); wake = undefined; return;
  }
  if (m.op === 'interrupt') {
    if (!exact(m, ['format', 'op', 'turn']) || m.turn !== lastTurn) throw new Error('interrupt');
    // The result may have crossed an interrupt in transit. A completed turn
    // cannot be interrupted retroactively, but still acknowledge the request.
    if (!active) { await emit('interrupted', m.turn, undefined, { completed: true }); return; }
    if (active.interrupt) throw new Error('interrupt');
    const turn = active;
    turn.interrupt = (async () => { await queryHandle.interrupt(); await emit('interrupted', turn.id, undefined, {}); })();
    await turn.interrupt; return;
  }
  if (m.op === 'close') {
    if (!exact(m, ['format', 'op'])) throw new Error('close');
    closing = true; wake?.(); queryHandle.close();
    await emit('closed', undefined, undefined, {});
    socket.end(() => process.exit(0)); return;
  }
  throw new Error('operation');
}
let commands = Promise.resolve(), queued = 0;
socket.on('data', chunk => {
  pending = Buffer.concat([pending, chunk]);
  if (pending.length > MAX_FRAME) { fail('command-limit'); return; }
  let at;
  while ((at = pending.indexOf(10)) >= 0) {
    const line = pending.subarray(0, at); pending = pending.subarray(at + 1);
    if (++queued > 8) { fail('command-queue-limit'); return; }
    commands = commands.then(() => command(JSON.parse(line.toString('utf8')))).catch(() => fail('invalid-command')).finally(() => { queued--; });
  }
});
socket.on('end', () => { closing = true; wake?.(); queryHandle?.close(); process.exit(0); });
socket.on('error', () => { closing = true; queryHandle?.close(); process.exit(1); });
