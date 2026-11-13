//! The exact files used by the session wrapper. No package manager runs in
//! the guest: the SDK entry point is self-contained and imports Node builtins.
//! Optional SDK integrations (custom MCP servers, browser/bridge APIs) are not
//! part of this profile. Changing any file changes the layer cache identity.

use crate::{Agent, layer};

/// The guest runtime's fixed directory.
pub const ROOT: &str = "/opt/kai/agents/session";
/// The Node executable inside the read-only layer.
pub const NODE_PATH: &str = "/opt/kai/agents/session/node";
/// The SDK wrapper inside the read-only layer.
pub const WRAPPER_PATH: &str = "/opt/kai/agents/session/wrapper.mjs";
/// Version of the SDK transport wrapper.
pub const FORMAT: &str = "kai-agent-session/1";
/// Exact SDK release paired with Claude Code.
pub const SDK_VERSION: &str = "0.3.323";
/// Exact Node release used to run the SDK.
pub const NODE_VERSION: &str = "22.22.0";

/// Claude paired with SDK 0.3.323; release-manifest checksum and byte length.
pub const CLAUDE: Agent = Agent {
    name: "claude-code",
    version: "2.1.323",
    length: 362_435_088,
    path: "/opt/kai/agents/session/claude",
    binary: vm::Artefact {
        name: "Claude Code 2.1.323",
        file: "claude-2.1.323",
        url: "https://storage.googleapis.com/claude-code-dist-86c565f3-f756-42ad-8dfa-d59b1c096819/claude-code-releases/2.1.323/linux-x64/claude",
        download_sha256: "604d29a6869409106b6a1110bdadcc8b72e8654a5a865d23258f938a696e348e",
        sha256: "604d29a6869409106b6a1110bdadcc8b72e8654a5a865d23258f938a696e348e",
        member: None,
        executable: true,
    },
};

/// Supporting artifacts, each downloaded and extracted by the existing fetcher.
pub const FILES: &[Agent] = &[
    Agent {
        name: "node",
        version: NODE_VERSION,
        length: 123_405_064,
        path: NODE_PATH,
        binary: vm::Artefact {
            name: "Node 22.22.0",
            file: "node-22.22.0",
            url: "https://nodejs.org/dist/v22.22.0/node-v22.22.0-linux-x64.tar.gz",
            download_sha256: "c33c39ed9c80deddde77c960d00119918b9e352426fd604ba41638d6526a4744",
            sha256: "1bec56ef7cfa9a76f3e0b7c0a87f220eb73f23102b9c0b4c7529a3f7c3ce7c31",
            member: Some("node-v22.22.0-linux-x64/bin/node"),
            executable: true,
        },
    },
    Agent {
        name: "claude-agent-sdk",
        version: SDK_VERSION,
        length: 1_420_658,
        path: "/opt/kai/agents/session/sdk.mjs",
        binary: vm::Artefact {
            name: "Claude Agent SDK 0.3.323",
            file: "claude-agent-sdk-0.3.323.mjs",
            url: "https://registry.npmjs.org/@anthropic-ai/claude-agent-sdk/-/claude-agent-sdk-0.3.323.tgz",
            download_sha256: "eac6f0fa22eaafc8c8e9355231aa953481899c2e7c647926b965457c26528e2f",
            sha256: "9350483cd0434c18c13745d7ca2630acd4ef29c6c2f0038b5a061f7e8c0fdb78",
            member: Some("package/sdk.mjs"),
            executable: false,
        },
    },
];

/// The wrapper source is part of the layer identity, not a mutable job file.
pub const INLINE: &[layer::Inline] = &[layer::Inline {
    path: WRAPPER_PATH,
    bytes: include_bytes!("wrapper.mjs"),
    version: FORMAT,
}];

/// Fetches and packs the pinned session runtime on an explicit user command.
pub async fn fetch(cache: &std::path::Path) -> Result<layer::Fetched, layer::Error> {
    layer::fetch_bundle(&CLAUDE, FILES, INLINE, cache, |_| {}).await
}

/// Requires a previously fetched, freshly verified session runtime.
pub async fn cached(cache: &std::path::Path) -> Result<layer::Fetched, layer::Error> {
    layer::cached_bundle(&CLAUDE, FILES, INLINE, cache).await
}
