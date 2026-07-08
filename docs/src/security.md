Security
========

### Lord commitment content (`/content/{bao_root}`)

Lord serves **decoded Carbonado commitment plaintext** at `/content/{bao_root}`
for **public** (even-format) commitments. Private commitments return **403
Forbidden** until authentication is implemented.

- **Content-Type** is inferred from decoded payload magic bytes (PNG, JPEG, WebP,
  GIF, PDF) or UTF-8 text fallback (`text/plain; charset=utf-8`); otherwise
  `application/octet-stream`.
- Responses include **`X-Content-Type-Options: nosniff`** to discourage MIME
  sniffing in browsers.
- Valid UTF-8 HTML served as `text/plain` is not rendered as HTML by modern
  browsers, but operators should still avoid hosting `lord server` on the same
  origin as privileged web apps (see cross-site scripting below).

Decoded payload size is capped at **32 MiB**; on-disk carbonado blobs are capped
before read at roughly **2× decoded + header** to limit memory exhaustion.

### Ord inscription content (removed in Lord)

Anyone could publish inscriptions, including arbitrary HTML, which `ord server`
served at `/content/<INSCRIPTION_ID>`,
`/r/undelegated-content/<INSCRIPTION_ID>`, and
`/r/sat/<SAT_NUMBER>/at/<INDEX>/content`. **Lord returns 410 Gone** for
inscription content routes.

This created potential security vulnerabilities, including cross-site scripting
and spoofing attacks.

Without mitigations, a domain hosting an `ord server` explorer instance should
be considered to be completely untrusted. The same-origin guidance applies to
**Lord** explorer deployments that serve commitment content.

### Cross-site Scripting

An attacker performs a cross-site scripting attack by injecting JavaScript into
a vulnerable website such that it is later served to a victim, causing the
malicious script to run in the victim's own browser with the same permissions
as any other script from that website.

Such a script could access private site resources or perform privileged actions
on the user's behalf, since it would be making requests on behalf of the user.

Cross-site scripting is *not* a concern for a vanilla `ord server` instance.
`ord server` does not serve any private resources or allow privileged actions
to be taken via the web interface, so although JavaScript can be injected using
an inscription, it does not run with any meaningful permissions.

However, if you serve requests to `ord server` from a domain which *also*
serves private resources or allow privileged actions, you may be vulnerable to
cross-site scripting attacks.

#### Example

- `ord server` is run under `https://example.com/ord`.

- A bitcoin exchange is run under `https://example.com/exchange`.

- Users can send bitcoin at `https://example.com/exchange/send`, with access
  controlled by a user's session cookie.

- An attacker publishes a malicious HTML inscription with ID `XYZ` which
  contains JavaScript that accesses `https://example.com/exchange/send` and
  sends them to an attacker-controlled address.

- When a user visits `https://example.com/ord/XYZ`, the malicious JavaScript
  requests `https://example.com/exchange/send`. Because the script executes on
  the user's behalf with the request including the user's session cookie, the
  server sends the user's bitcoin to the attacker.

To prevent this, do not make `ord server` available on the same domain and port
as another web service which is vulnerable to cross-site scripting.

### Spoofing

If `ord server` is run at a well known domain, for example, `ordinals.com`, an
attacker could publish a malicious inscription that attempts to trick users
into thinking that it represents the owners of the domain. For example, by
publishing a mint page to induce users to send bitcoin. Additionally, the
[History API](https://developer.mozilla.org/en-US/docs/Web/API/History_API) can
be used to change the path without triggering a page reload, so inscription
content can appear to the user under URL that suggests it was published by the
site owners.

#### Example

- `ord server` is run at `https://example.com`, by popular and well-respected
  Example Corporation.

- An attacker publishes a malicious HTML inscription with ID `XYZ` which
  displays a mint page, and changes the URL path using the History API to
  `/mint`.

- A user clicks a link to `https://example.com/inscription/XYZ` which displays
  the attacker's mint page, with the apparent URL of
  `https://example.com/mint`.

- The user thinks that this is a potentially lucrative mint run by popular and
  well-respected Example Corporation and sends bitcoin to the mint address.

To prevent this, avoid giving users the impression that a domain hosting an
`ord` explorer can be trusted for anything other than the content of the `ord`
explorer itself.
