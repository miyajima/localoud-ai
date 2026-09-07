"""Loopback-only, file-backed MCP fixture. Never connects to real OrgBrain."""
import json
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

storage = Path(sys.argv[1])
class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass
    def do_POST(self):
        if self.headers.get('CF-Access-Client-Id') != 'fixture-id' or self.headers.get('CF-Access-Client-Secret') != 'fixture-secret':
            self.send_error(403); return
        request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        args = request['params']['arguments']
        if args['tenant_id'] != 'fixture':
            self.send_error(403); return
        saved = json.loads(storage.read_text()) if storage.exists() else {}
        name = request['params']['name']
        if name == 'orgbrain_memories_upsert':
            for item in args['items']:
                assert item['project_id'] == 'fixture-project'
                saved[item['external_key']] = item
            storage.write_text(json.dumps(saved))
            result = {'upserted': len(args['items'])}
        elif name == 'orgbrain_memories_search':
            assert args['project_id'] == 'fixture-project'
            result = {'results': [{'id': key, 'content_preview': item['content'][:1000]} for key, item in saved.items() if args['q'].lower() in item['content'].lower()][:args['limit']]}
        elif name == 'orgbrain_decision_memories_search':
            result = {'results': []}
        else:
            self.send_error(400); return
        body = json.dumps({'jsonrpc': '2.0', 'id': request['id'], 'result': {'content': [{'type': 'text', 'text': json.dumps(result)}]}}).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers(); self.wfile.write(body)
server = HTTPServer(('127.0.0.1', 0), Handler)
print(server.server_address[1], flush=True)
server.serve_forever()
