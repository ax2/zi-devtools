// Optional development fixture only. The desktop application remains Rust-only.
import http from 'node:http';
import { randomUUID } from 'node:crypto';
import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js';
import { StreamableHTTPServerTransport } from '@modelcontextprotocol/sdk/server/streamableHttp.js';
import * as z from 'zod/v4';
import { requireBearerAuth } from '@modelcontextprotocol/sdk/server/auth/middleware/bearerAuth.js';
import { InvalidTokenError } from '@modelcontextprotocol/sdk/server/auth/errors.js';

const sessions = new Map();
const jsonResponse = process.argv[2] === 'json';
const authenticated = process.argv[3] === 'auth';
const lifecycle = authenticated && process.argv[4] === 'lifecycle';
const tlsDiscovery = lifecycle && process.argv[5] === 'tls';
const revoked = new Set();
let revocationRequests = 0;
let deniedRequests = 0;
let rotated = false;
const auth = requireBearerAuth({
  requiredScopes: ['fixture:read'],
  verifier: { async verifyAccessToken(token) {
    if (revoked.has(token)) throw new InvalidTokenError('Synthetic token revoked');
    if (token === 'zi-sdk-synthetic-restricted')
      return { token, clientId: 'synthetic-client', scopes: [], expiresAt: Date.now() / 1000 + 120 };
    if (token === 'zi-sdk-synthetic-expired')
      return { token, clientId: 'synthetic-client', scopes: ['fixture:read'], expiresAt: Date.now() / 1000 - 10 };
    if (token === 'zi-sdk-synthetic-new') rotated = true;
    if (token !== 'zi-sdk-synthetic-new' && (token !== 'zi-sdk-synthetic-old' || rotated))
      throw new InvalidTokenError('Synthetic token rejected');
    return { token, clientId: 'synthetic-client', scopes: ['fixture:read'], expiresAt: Date.now() / 1000 + 120 };
  } },
});
// Node response adapter for the SDK's documented Express middleware surface.
async function authorize(req, res) {
  res.set = (key, value) => { res.setHeader(key, value); return res; };
  res.status = code => { res.statusCode = code; return res; };
  res.json = value => { res.setHeader('Content-Type', 'application/json'); res.end(JSON.stringify(value)); return res; };
  let accepted = false;
  await auth(req, res, () => { accepted = true; });
  return accepted;
}
function createServer() {
  const server = new McpServer({ name: 'Zi SDK interoperability fixture', version: '1.0.0' });
  server.registerTool('echo', { inputSchema: { text: z.string() } }, async ({ text }) => ({ content: [{ type: 'text', text }] }));
  server.registerResource('Guide', 'fixture://guide', { mimeType: 'text/plain' }, async () => ({ contents: [{ uri: 'fixture://guide', text: 'SDK synthetic guide' }] }));
  server.registerPrompt('summary', { argsSchema: {} }, async () => ({ messages: [{ role: 'user', content: { type: 'text', text: 'SDK synthetic prompt' } }] }));
  return server;
}
const listener = http.createServer(async (req, res) => {
  try {
    // Explicit synthetic-only lifecycle control, bound to this loopback listener.
    if (lifecycle && req.url === '/fixture/revoke' && req.method === 'POST') {
      const chunks = []; let size = 0;
      for await (const chunk of req) {
        size += chunk.length;
        if (size > 1024) { res.writeHead(413).end(); return; }
        chunks.push(chunk);
      }
      const { token } = JSON.parse(Buffer.concat(chunks).toString('utf8'));
      if (!['synthetic-refresh-new', 'zi-sdk-synthetic-new'].includes(token)) {
        res.writeHead(400).end(); return;
      }
      revocationRequests++;
      if (token === 'zi-sdk-synthetic-new') revoked.add(token);
      res.writeHead(200).end(); return;
    }
    if (lifecycle && req.url === '/fixture/finish' && req.method === 'POST') {
      const success = revocationRequests === 2 && revoked.has('zi-sdk-synthetic-new') && deniedRequests === (tlsDiscovery ? 2 : 1) && sessions.size === 0;
      res.writeHead(success ? 200 : 409).end();
      listener.close(); return;
    }
    if (req.url !== '/mcp') { res.writeHead(404).end(); return; }
    if (authenticated && !await authorize(req, res)) { deniedRequests++; return; }
    const id = req.headers['mcp-session-id'];
    let transport = sessions.get(id);
    let body;
    if (req.method === 'POST') {
      const chunks = [];
      let size = 0;
      for await (const chunk of req) {
        size += chunk.length;
        if (size > 256 * 1024) { res.writeHead(413).end(); return; }
        chunks.push(chunk);
      }
      body = JSON.parse(Buffer.concat(chunks).toString('utf8'));
      if (!transport && !id && body.method === 'initialize') {
        transport = new StreamableHTTPServerTransport({
          sessionIdGenerator: randomUUID,
          enableJsonResponse: jsonResponse,
          onsessioninitialized: key => sessions.set(key, transport),
        });
        transport.onclose = () => sessions.delete(transport.sessionId);
        await createServer().connect(transport);
      }
    }
    if (!transport) { res.writeHead(404).end(); return; }
    if (req.method === 'DELETE' && !lifecycle) res.on('finish', () => listener.close());
    await transport.handleRequest(req, res, body);
  } catch {
    if (!res.headersSent) res.writeHead(500).end();
  }
});
listener.listen(0, '127.0.0.1', () => console.log(`http://127.0.0.1:${listener.address().port}/mcp`));
const deadline = setTimeout(() => { listener.close(); process.exitCode = 1; }, lifecycle ? 120000 : 60000);
deadline.unref();
