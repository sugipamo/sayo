# Playground Deployment

English | [日本語](deployment.ja.md)

The Sayo Playground is prepared for Cloudflare Pages, but this repository does not create a Pages project or publish a
deployment. Publication is a separate maintainer operation.

## Cloudflare Pages settings

Connect `https://github.com/sugipamo/sayo` through Cloudflare Pages Git integration and use these settings:

| Setting | Value |
|---|---|
| Framework preset | None |
| Root directory | Repository root |
| Build command | `npm run build:pages` |
| Build output directory | `playground` |
| Environment variables | None required |

Choose the repository's production branch when the Pages project is created. The initial public address should use the
project's generated `*.pages.dev` subdomain. A custom domain can be attached later without changing Playground paths.

The build command installs a minimal latest-stable Rust toolchain when Cargo is unavailable, adds the
`wasm32-unknown-unknown` target, installs the exact `wasm-bindgen-cli` version expected by the repository, and delegates to
the ordinary Wasm build. Generated bindings remain in the ignored `playground/pkg/` directory and become part of the
Pages output without being committed.

## Verify before publishing

Run the Pages build locally:

```sh
npm run build:pages
```

Then run the browser suite:

```sh
npm run test:browser
```

After the first deployment, verify the generated `pages.dev` address manually before adding it to the root README. Do not
add a speculative URL: the exact subdomain is assigned when the Cloudflare Pages project is created.

## Deployment boundary

Git integration owns automatic builds and deployments after the project is connected. Sayo does not currently require
Pages Functions, Workers, server-side state, secrets, or build-time application configuration. Shared programs remain in
the URL fragment and are not sent to Cloudflare as application data.

Creating the Cloudflare project, authorizing GitHub, selecting the production branch, and performing the first deployment
are intentionally left for the publication step.
