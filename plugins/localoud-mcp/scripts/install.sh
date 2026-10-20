#!/bin/sh
# Build localoud-mcp and print Codex / Claude Code registration snippets.
set -eu
cd "$(dirname "$0")/../../.."

cargo build --release -p localoud-mcp
BIN="$PWD/target/release/localoud-mcp"
mkdir -p ~/.local/bin
ln -sf "$BIN" ~/.local/bin/localoud-mcp

CONFIG=~/.config/localoud/mcp-projects.json
if [ ! -f "$CONFIG" ]; then
  mkdir -p ~/.config/localoud
  cp plugins/localoud-mcp/mcp-config.example.json "$CONFIG"
  echo "wrote example config to $CONFIG -- edit roots before use"
fi

case ":$PATH:" in
  *":$HOME/.local/bin:"*) ;;
  *) echo 'NOTE: add ~/.local/bin to PATH' ;;
esac

echo
echo "Codex CLI (~/.codex/config.toml):"
echo '  [mcp_servers.localoud]'
echo '  command = "localoud-mcp"'
echo "  args = [\"--config\", \"$CONFIG\"]"
echo
echo "Claude Code (project scope):"
echo "  cp plugins/localoud-mcp/.mcp.json <your-project>/.mcp.json"
echo "  # then allow-list the server when Claude Code prompts"
echo "Claude Code (user scope):"
echo "  claude mcp add localoud -- localoud-mcp --config $CONFIG"
echo
echo "HTTP (optional, loopback only):"
echo "  LOCALOUD_MCP_TOKEN=\$(openssl rand -hex 32) localoud-mcp --config $CONFIG --http 127.0.0.1:8792"
