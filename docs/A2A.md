# A2A support

Localoud uses A2A v1 as the envelope at explicit agent boundaries. The envelope is an A2A `Message`; Localoud's bounded context, review, or research payload is carried in a structured `data` Part with a Localoud media type. `contextId`, `taskId`, and `referenceTaskIds` preserve workflow identity and dependency references without copying an implicit parent conversation.

This follows the current [A2A v1 specification](https://a2a-protocol.org/latest/specification/). A2A standardizes discovery, messages, parts, tasks, artifacts, transports, and lifecycle semantics. It does not define the application-specific fields of a coding context capsule, so those fields remain a versioned Localoud extension.

## Compatibility boundary

A2A is not used unchanged as the wire request body for Codex app-server, OpenAI Chat Completions/Responses, Anthropic Messages, or a local completion endpoint. Localoud validates the envelope, then each provider adapter renders it as text or maps it into that provider's native request format. Provider-native conversation state also remains native within one provider segment.

This means Localoud can use one A2A handoff contract without requiring those providers to accept raw A2A JSON. Replacing the provider adapters or every internal persistence type with A2A would break useful provider features and is intentionally out of scope.

The following explicit boundaries now use A2A Messages:

- generic task-to-worker context capsules;
- autonomous implementation-step capsules;
- autonomous final-review capsules;
- staged research handoffs.
- validated conversation handoffs when an API-provider session moves to a fresh Codex thread.

Conversation classification is local-first but not trusted. The local model may only propose exact quotes and relationships. Localoud's existing deterministic handoff code validates those proposals against the stored visible transcript and wraps the accepted selection in a normal context capsule. The full transcript is not sent as a fallback.

## Local HTTP+JSON interface

When the existing read-only MCP service is enabled, the same loopback listener exposes a bounded A2A v1 HTTP+JSON interface:

- Agent Card: `http://127.0.0.1:8792/.well-known/agent-card.json`
- preferred interface base: `http://127.0.0.1:8792/a2a/v1`
- send message: `POST /a2a/v1/message:send`

The Agent Card is locally discoverable without a bearer token. Operations require the opaque bearer token stored in macOS Keychain, an exact loopback Host/Origin policy, `Content-Type: application/a2a+json`, `Accept: application/a2a+json`, and `A2A-Version: 1.0`.

The advertised `localoud-read-evidence` skill accepts exactly one `ROLE_USER` data Part with media type `application/vnd.localoud.read-request+json`:

```json
{
  "message": {
    "messageId": "unique-message-id",
    "role": "ROLE_USER",
    "parts": [{
      "data": {
        "skillId": "localoud-read-evidence",
        "tool": "project_list",
        "arguments": {}
      },
      "mediaType": "application/vnd.localoud.read-request+json"
    }]
  },
  "configuration": {
    "acceptedOutputModes": ["application/json"]
  }
}
```

The response is an A2A `SendMessageResponse` containing a direct `ROLE_AGENT` Message. It does not create or continue an A2A Task, run commands, edit files, or expose projects the user did not publish.

## Current conformance claim

Implemented now: v1 Message/Part shapes, Agent Card discovery, HTTP+JSON `message:send`, version negotiation, bearer authentication, declared media modes and extension, structured direct-message responses, and bounded read-only execution.

Not implemented or claimed: streaming, push notifications, remote HTTPS publication, A2A Task lifecycle endpoints, arbitrary remote-agent execution, signed Agent Cards, or certification against the A2A interoperability test suite. A production remote interface must publish an accurate HTTPS URL in its Agent Card and be tested separately.
