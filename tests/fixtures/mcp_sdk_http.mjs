// Optional development fixture only. The desktop application remains Rust-only.
import http from 'node:http';
import { randomUUID } from 'node:crypto';
import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js';
import { StreamableHTTPServerTransport } from '@modelcontextprotocol/sdk/server/streamableHttp.js';
import * as z from 'zod/v4';

const sessions = new Map();
const jsonResponse = process.argv[2] === 'json';
function createServer() {
  const server = new McpServer({ name: 'Zi SDK interoperability fixture', version: '1.0.0' });
  server.registerTool('echo', { inputSchema: { text: z.string() } }, async ({ text }) => ({ content: [{ type: 'text', text }] }));
  server.registerResource('Guide', 'fixture://guide', { mimeType: 'text/plain' }, async () => ({ contents: [{ uri: 'fixture://guide', text: 'SDK synthetic guide' }] }));
  server.registerPrompt('summary', { argsSchema: {} }, async () => ({ messages: [{ role: 'user', content: { type: 'text', text: 'SDK synthetic prompt' } }] }));
  return server;
}
const listener = http.createServer(async (req, res) => {
  try {
    if (req.url !== '/mcp') { res.writeHead(404).end(); return; }
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
    if (req.method === 'DELETE') res.on('finish', () => listener.close());
    await transport.handleRequest(req, res, body);
  } catch {
    if (!res.headersSent) res.writeHead(500).end();
  }
});
listener.listen(0, '127.0.0.1', () => console.log(`http://127.0.0.1:${listener.address().port}/mcp`));
const deadline = setTimeout(() => { listener.close(); process.exitCode = 1; }, 60000);
deadline.unref();
