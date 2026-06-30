# Cntx Code - Growth, Publishing, And Monetization Guide

This file is internal strategy. It is excluded from the published crate and binary
(see `Cargo.toml` `exclude`), so consumers never see it. It covers how to get users,
how to run the X account and earn the gold verification tick, how to build and
publish the project, and how to monetize after you have free traction.

Primary domain: <https://cntxcode.com>
Redirect domain: <https://cntx.codes>

---

## 1. SEO (cntxcode.com first, cntx.codes redirects)

Make cntxcode.com the canonical site and have cntx.codes 301-redirect every path to
the matching path on cntxcode.com. Point both at the same origin, then canonicalize.

On-page SEO basics:

- One `<h1>` per page containing the primary keyword: "BYOK AI coding assistant",
  "token-efficient coding CLI", "Claude Code alternative".
- `<title>` under 60 chars, e.g. "Cntx Code - BYOK Token-Efficient AI Coding CLI".
- Meta description under 155 chars mentioning BYOK, token efficiency, and CLI.
- `<link rel="canonical" href="https://cntxcode.com/...">` on every page.
- Structured data: `SoftwareApplication` JSON-LD with name, operatingSystem,
  applicationCategory "DeveloperApplication", and offers (free).
- Sitemap at `https://cntxcode.com/sitemap.xml`; submit in Google Search Console
  and Bing Webmaster Tools. Verify both domains; set cntxcode.com as canonical.
- Fast load: static HTML, no client-side rendering for content. Lighthouse > 95.

Landing pages to write (each targeting one intent):

- `/` - the product pitch and install command
- `/docs` - mirror of the repo docs (or link to GitHub)
- `/byok` - "bring your own keys" explained (keyword: BYOK AI assistant)
- `/token-efficient` - the optimization story (keyword: token efficient LLM CLI)
- `/ollama-cloud` - Ollama Cloud + Pro setup (keyword: Ollama Cloud CLI)
- `/claude-code-alternative` - honest comparison page (keyword: Claude Code
  alternative). Be accurate, do not claim parity with features you do not have.

Content marketing that compounds:

- A `/blog` with evergreen posts: "How to cut LLM token costs as a developer",
  "BYOK vs managed AI coding tools", "Ollama Cloud Pro from the terminal".
- Each post links back to a feature page with descriptive anchor text.
- Cross-post to dev.to and Hashnode with a canonical back to cntxcode.com.

Backlinks:

- List Cntx Code in awesome-lists (awesome-cli, awesome-llm, awesome-ai-tools).
- Submit to free directories: ToolHunt, Futurepedia, There's An AI For That.
- Answer relevant Stack Overflow / Reddit questions with genuine help, link only
  when it truly answers the question.

Avoid black-hat tactics. Slow, real backlinks outrank purchased ones and survive
algorithm updates.

---

## 2. Getting Users (Distribution)

Order of channels by ROI for a developer CLI:

1. **GitHub** - the storefront. Pin a tight README, a 60-second demo GIF, a one-line
   install, and a "Quick Start" that works in under 2 minutes. Star CTA. Releases
   with checksums. Topics: `cli`, `ai`, `coding-assistant`, `byok`, `llm`,
   `rust`. Project URL set to `https://cntxcode.com`.
2. **Homebrew tap** - the single highest-conversion channel for macOS devs. Once
   `brew install cntx-code && cntx` works, install friction is near zero (see
   publishing below).
3. **Hacker News "Show HN"** - one shot, make it count. Lead with the problem
   ("most coding assistants waste tokens; Cntx Code optimizes prompts and routes by
   size"), show a real before/after token number, and be in the comments for 4 hours.
4. **Reddit** - `r/programming`, `r/rust`, `r/LocalLLaMA`, `r/programmingtools`,
   `r/ChatGPTCoding`. Post in the spirit of each community; r/LocalLLaMA loves the
   Ollama Cloud angle. Read each subreddit's self-promo rules first.
5. **X / LinkedIn** - build in public (see section 3).
6. **Discord / Slack communities** - dev tool discords, Rust channels, Ollama
   community. Help people, mention the tool where it fits.
7. **Dev.to / Hashnode / HackerNoon** - one technical deep dive per week.
8. **Product Hunt** - launch once the install is frictionless and the demo is
   polished. Aim for a Tuesday or Wednesday launch.

Traction signals to track weekly: GitHub stars, brew installs (if measurable),
`cntx --version` install guides run, landing page signups, X followers.

First 100 users playbook:

- Ship the Homebrew tap and a one-line install before any launch post.
- Record a 60-90s demo showing a real token saving and the sandbox.
- Line up 5-10 friendly devs to try it and give feedback before the public launch.
- Launch on HN + Reddit + X on the same morning so traffic compounds.
- Be relentlessly helpful in every reply for the first 48 hours.

First 1,000 users playbook:

- Start with 50 hand-picked developers who already feel pain from AI coding tool
  costs: Rust, local-LLM, BYOK, and agent-tooling communities. Ask each for a
  10-minute install call or async feedback.
- Turn every sharp piece of feedback into a visible release note. Early users want
  proof that the project is alive more than they want polish.
- Make the public promise narrow: "BYOK, token-efficient coding CLI with sandboxed
  file writes." Avoid claiming parity with larger agent tools until the autonomous
  loop is mature.
- Publish one concrete benchmark page before broad launch: same prompt, before and
  after optimization, tokens saved, model routed, and total request cost.
- Use GitHub as the conversion point. Every post should lead to a repo with
  one-line install, screenshots or a 60-second demo, and a working quick start.
- Run three launch waves instead of one: private beta to 50 users, public
  developer launch to 250 users, then a comparison/benchmark launch after fixes.
- Build integrations only when they unlock distribution: Homebrew first, then
  crates.io, then docs/examples for Ollama Cloud, Anthropic, and OpenAI-compatible
  gateways.
- Ask for a star only after the user has successfully run the first prompt. The
  best CTA is printed at the end of a successful install guide or demo, not before.
- Collect emails lightly on cntxcode.com with a "release notes and launch invites"
  form. Do not put the CLI behind an account.
- Weekly target until 1,000: 1 release, 1 technical post, 5 helpful replies per
  day, 10 direct user conversations, and a public metric update.

---

## 3. Optimizing The X Account And Earning The Gold Tick

Account setup:

- Handle ideally `@cntxcode` (or `@cntx_code`). Reserve `@cntxcodes` too.
- Name: "Cntx Code". Bio: one line of value, one line of credibility, link to
  cntxcode.com. Example: "BYOK, token-efficient AI coding CLI. Bring your own
  keys. Sandbox-safe. Rust." then the domain.
- Profile image: the logo on a contrasting background, legible at 48x48. Header:
  the product in one screenshot or a bold value statement.
- Pinned post: the 60s demo + one-line install.

Content cadence (aim for 1-2 posts per day early):

- **Build in public** - share one concrete thing per day: a token saving number, a
  new provider adapter, a sandbox rule. Specifics beat platitudes.
- **Threads** - one deep thread per week (a real workflow: set up Ollama Cloud Pro,
  cut a prompt from 8k to 1.2k tokens). Threads get the most reach.
- **Demos** - short screen recordings of the CLI in action. Motion sells tools.
- **Comparisons** - honest, specific comparisons ("here is where Cntx Code is
  different: you own the keys, you see the optimization").
- **Replies** - reply to anyone talking about token costs, BYOK, Ollama, or Claude
  Code alternatives. Replies grow followers faster than posts early on.

Engagement rules:

- Reply within the hour for the first few months. Recency compounds.
- Quote-relevant posts from Ollama, Anthropic, OpenAI, and dev tool accounts with
  genuine added value, not just "nice".
- Never engagement-bait or buy followers. The algorithm and advertisers detect it.

The gold tick (X Verified Organizations):

- The gold check is for **organizations/businesses**, not individuals. It requires
  an **X Verified Organizations** subscription on the business account, not a
  Premium personal subscription.
- Tiers (verify current pricing at <https://help.x.com/en/using-x/verified-organizations>,
  prices change): the base Verified Organizations plan (around $200/month) gives the
  gold badge and basic verification; the higher tier adds affiliate badges and more.
- Apply at <https://verified.x.com> with the business name, a matching domain
  (use cntxcode.com - X checks domain ownership), and verification documents. Have
  the company entity, EIN/registration, and a professional email on the domain
  ready.
- Pragmatic path: do not buy the gold tick on day one. Spend the first months
  getting real traction (stars, users, content). Buy it when you are about to do
  partnerships, raise money, or want the affiliate-badge feature for team
  accounts. The badge signals legitimacy but does not create it.
- Faster credibility substitute until then: get the blue check via X Premium on the
  personal founder account, and grow the @cntxcode account organically with the
  "build in public" cadence above.

---

## 4. Building And Publishing

### Build

```bash
cargo build --release
```

The release binary is at `target/release/cntx`.

Verify before publishing:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

### Publish to crates.io

```bash
cargo login            # one time, with your crates.io token
cargo publish --dry-run
cargo publish
```

`Cargo.toml` excludes all markdown (including this file) and any secrets from the
published package, so consumers get a clean crate. After publishing, users install
with `cargo install cntx`.

### Cross-compile release binaries

Use a release workflow (GitHub Actions) to build for macOS (arm64, x86_64) and
Linux (x86_64). For each target, produce a `.tar.gz` and a SHA256 checksum. Attach
them to a GitHub Release. Example matrix:

- `aarch64-apple-darwin`
- `x86_64-apple-darwin`
- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`

Sign and notarize macOS builds (notarytool) so Gatekeeper does not warn users.

### Homebrew tap (highest-conversion channel)

Create a tap repo, e.g. `cntx-code/homebrew-tap`, with `Formula/cntx-code.rb`:

```ruby
class CntxCode < Formula
  desc "BYOK, token-efficient AI coding assistant"
  homepage "https://cntxcode.com"
  url "https://github.com/cntx-code/cntx/releases/download/v0.1.0/cntx-aarch64-apple-darwin.tar.gz"
  sha256 "REPLACE_WITH_CHECKSUM"
  version "0.1.0"
  license "MIT"

  def install
    bin.install "cntx"
  end

  test do
    assert_match "Cntx Code", shell_output("#{bin}/cntx --version")
  end
end
```

Users install with:

```bash
brew tap cntx-code/tap
brew install cntx-code
```

Update the formula's `url`/`sha256`/`version` on every release. Consider
`homebrew-releaser` or `cargo-dist` to automate this.

### Other package channels (later)

- AUR (`cntx-code` and `cntx-code-bin`) for Arch users.
- Scoop / Winget for Windows once Windows is supported.
- Nix flake for the Nix crowd.

### First run on any machine

After install, the tool self-configures:

```bash
cntx api-key add --provider anthropic --value sk-ant-...
cntx endpoint --new --name work --provider anthropic --api-key-env ANTHROPIC_API_KEY
cntx endpoint --set-primary work
cntx --refresh-models
cntx "explain this repository"
```

The secrets file is created automatically on first boot with `0600` permissions. No
manual setup is required on a fresh machine.

### Versioning and changelog

- Tag releases with semantic versioning (`v0.1.0`).
- Maintain a `CHANGELOG.md` (this can be published; it is not excluded if you want
  it visible, or keep it internal).
- Write a short release note for each version and post it to X.

---

## 5. Monetization (After Free Traction)

Keep the core CLI free and open source. Monetization layers on top once you have
real, retained users. Do not gate the BYOK workflow - that is the product's reason
to exist.

Tiers that fit a developer CLI:

1. **Free / Open source** - the CLI, all adapters, the sandbox, built-in MCPs,
   BYOK. No limits, no account required. This is the growth engine. Keep it
   generous forever.
2. **Cntx Cloud (managed relay)** - optional hosted relay for users who do not want
   to manage keys: a single `cntx` login, provider routing, usage dashboards, and
   consolidated billing. Charge per request or a flat seat. The CLI falls back to
   direct BYOK if the cloud is unavailable, so users are never locked in.
3. **Team plan** - shared endpoints, team skills, audit logs, SSO, central billing.
   Per-seat pricing. This is where most revenue lives for dev tools.
4. **Enterprise** - self-hosted relay, SSO/SAML, compliance exports, priority
   support, on-prem deployment, custom provider integrations. Annual contracts.

Adjacent revenue:

- **Managed Headroom / token-saving proxy** - offer a hosted compression proxy that
   cuts token costs across all their providers for a percentage of savings or a
   flat fee. Aligns incentives (you save them money, you take a cut).
- **Premium skills and prompt packs** - curated, maintained skill and prompt
   libraries for specific stacks (Rails, React, embedded, etc.).
- **Support and consulting** - paid priority support, onboarding, and custom
   integrations for teams.
- **Sponsorships** - GitHub Sponsors for individuals; paid provider spotlights in
   docs (clearly labeled) once traffic is meaningful.

Pricing guidance:

- Price per developer seat, not per token. Developers hate unpredictable bills.
- Keep the free tier genuinely useful; the upgrade must be about convenience and
  team features, not paywalls on core capability.
- Publish simple pricing. Three tiers max on the pricing page.

When to start:

- Do not build the cloud relay until you have retention signal (users coming back
  weekly without prompting).
- Start monetization conversations only after 1,000+ active installs and a clear
  pattern of team usage requests. Premature paywalls kill organic growth.
- The earliest revenue will likely be support/consulting and sponsorships, not
  SaaS - that is normal for a CLI-first project.

---

## 6. What To Do This Week

1. Finalize the GitHub repo: README, demo GIF, one-line install, topics, project URL.
2. Stand up cntxcode.com with the landing pages and canonical/redirect setup.
3. Cut the first GitHub Release with macOS + Linux binaries and checksums.
4. Publish the Homebrew tap and verify `brew install cntx-code` works clean.
5. Record the 60s demo and pin it on X.
6. Prepare the HN "Show HN" title and a genuine, technical first comment.
7. Schedule the launch week: HN + Reddit + X on one morning, dev.to deep dive
   midweek, follow-up demo end of week.

After launch, switch to the weekly rhythm: one deep thread, one release or feature,
relentless replies, and a weekly traction review.
