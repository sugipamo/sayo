# Playground Deployment

English | [日本語](deployment.ja.md)

The Sayo Playground is published at <https://sayo-lang.pages.dev/> through the `sayo-lang` Cloudflare Pages project.

## Cloudflare Pages settings

The current deployment uses Wrangler Direct Upload. The equivalent build settings are:

| Setting | Value |
|---|---|
| Framework preset | None |
| Root directory | Repository root |
| Build command | `npm run build:pages` |
| Build output directory | `playground` |
| Environment variables | None required |

The production branch is `main`. A custom domain can be attached later without changing Playground paths.

The build command installs a minimal latest-stable Rust toolchain when Cargo is unavailable, adds the
`wasm32-unknown-unknown` target, installs the exact `wasm-bindgen-cli` version expected by the repository, and delegates to
the ordinary Wasm build. Generated bindings remain in the ignored `playground/pkg/` directory and become part of the
Pages output without being committed.

## Publish and verify

Run the Pages build locally:

```sh
npm run build:pages
```

Then run the browser suite:

```sh
npm run test:browser
```

Deploy the generated Playground with the current Git commit attached:

```sh
npx wrangler pages deploy playground --project-name sayo-lang --branch main --commit-hash "$(git rev-parse HEAD)"
```

Verify <https://sayo-lang.pages.dev/> after the upload completes.

## Deployment boundary

Deployment is currently an explicit maintainer operation. Sayo does not require Pages Functions, Workers, server-side
state, secrets, or build-time application configuration. Shared programs remain in the URL fragment and are not sent to
Cloudflare as application data. Git integration may replace Direct Upload later if automatic deployment becomes useful.
