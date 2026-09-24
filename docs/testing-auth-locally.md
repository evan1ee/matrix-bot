# Testing passkey auth locally

Everything below runs on `localhost` with no HTTPS, no hardware key, and no
production deployment.

## One-time setup

`.env` already has these. They are the only new configuration:

```bash
WEBAUTHN_RP_ID=localhost
WEBAUTHN_ORIGIN=http://localhost:5173
```

**`WEBAUTHN_ORIGIN` must be the port you actually browse on.** It is checked
against the `Origin` header of every state-changing request, so if you run the
Vite dev server on `:5173` and leave this at `:8080`, every write fails with:

```
{"error":"This request did not come from this site","code":401}
```

`WEBAUTHN_RP_ID` is a bare hostname and ignores ports, so one value of
`localhost` covers both servers. Cookies ignore ports too, so a session opened
on either port works on the other.

## Running it

Two terminals, both from the repo root:

```bash
cargo run                 # bot + API on :8080
npm --prefix frontend run dev   # SPA on :5173, proxies /api to :8080
```

Then open <http://localhost:5173>.

## Passkeys without a hardware key

Chrome and Edge can fake an authenticator, which is faster than Touch ID and
lets you reset between runs:

1. DevTools → ⋮ → **More tools** → **WebAuthn**
2. Tick **Enable virtual authenticator environment**
3. **Add** an authenticator with:
   - Protocol: **ctap2**
   - Transport: **internal**
   - **Supports resident keys** ✅ ← required; sign-in is usernameless and
     will not work without it
   - **Supports user verification** ✅ ← required; registration asks for it

Registered credentials appear in that panel and can be deleted individually,
which is how you test "I lost my passkey" without losing anything.

Safari and Firefox will use the real platform authenticator (Touch ID, Windows
Hello). That works too — it is just harder to reset.

## The flows

### Register and sign in

1. Open Settings → **Security** is empty until you have an account.
2. Sign in dialog → **Create an account instead** → display name → **Create
   passkey**.
3. Save the recovery codes it shows. They are not shown again.
4. Sign out, then **Sign in** — one button, no username. The browser offers the
   credential and that is the whole flow.

If registration is invite-gated, set `API_REGISTRATION_SECRET` in `.env` and
enter it in the invite field.

### Link a Matrix account

Needs the bot actually logged in and syncing (check the `cargo run` log).

1. Settings → Security → **Link a Matrix account** → a code appears
   (`!link A3F9-K2QX`).
2. From the Matrix account you want to link, send that line to the bot **in a
   direct message**.
3. The bot replies, and the account shows up in the Security panel.

Two things to try deliberately:

- **Send it in a shared room.** The bot refuses and cancels the code — it was
  visible to everyone there.
- **Link an MXID that has used the bot before.** Instead of linking, you get a
  count of its existing data and a confirmation prompt in Settings → Security.
  Nothing moves until you agree.

### Step-up (sensitive actions asking for a passkey)

Anything that changes how the account is reached needs a passkey assertion from
the last five minutes. To test the expiry without waiting:

```bash
psql "$DATABASE_URL" -c \
  "UPDATE sessions SET last_verified_at = NOW() - INTERVAL '10 minutes';"
```

Now click **Sign out other devices** or **Generate new codes**. The server
returns 403, the UI prompts for your passkey, and the action retries — a
sensitive action occasionally asking for your passkey is the intended
experience, not an error.

### Account recovery

`!recover` in a DM to the bot starts a 24-hour delayed recovery. To avoid
waiting a day:

```bash
psql "$DATABASE_URL" -c \
  "UPDATE recovery_requests SET valid_after = NOW() - INTERVAL '1 minute';"
```

Also worth testing the other direction: after `!recover`, open Settings →
Security in a signed-in tab. There is a red banner with a **Cancel recovery**
button, and cancelling deliberately does *not* require a passkey — you may be
racing the clock on a phone.

### Enroll a passkey from Matrix chat (`!passkey add` / `!web`)

If your account has **no passkey yet** (for example a Matrix-only account), you can attach its first passkey directly from chat without registering an orphan web account first:

1. Send `!passkey add` (or `!web`) to the bot **in a direct message**.
2. The bot replies with a single-use 10-minute link (e.g. `http://localhost:5173/?enroll=<token>`).
3. Opening the link triggers the passkey registration ceremony and immediately links the new passkey to your existing Matrix account.
4. Additional passkeys are added with the **+ Add a passkey** button in Settings → Security. Once the account has a passkey, `!passkey add` refuses and points at the web app and `!recover`, and an older link stops working too.

### List passkeys from Matrix (`!passkeys`)

- **`!passkeys`** (or `!passkey list`): Lists all passkeys attached to your account with index numbers (e.g. `#1 Touch ID`, `#2 YubiKey`). Listing is read-only.

### The Matrix ceiling

These actions still refuse and point at the web app, because they require web-only step-up assertion or direct browser downloads:

```
!export      !sessions      !recoverycodes      !unlink      !passkey rm
```

## Resetting

```bash
psql "$DATABASE_URL" -c "DROP SCHEMA public CASCADE; CREATE SCHEMA public;"
cargo run   # re-applies migrations on boot; fresh database, fresh accounts
```

Delete the virtual authenticator in the WebAuthn panel to drop the passkeys
that pointed at it. Deleting one without the other leaves credentials that
reference an account that no longer exists — harmless, but sign-in fails in a
confusing way.

## Going to production

Set both variables to the real host before deploying:

```bash
WEBAUTHN_RP_ID=bot.example.com
WEBAUTHN_ORIGIN=https://bot.example.com
ENVIRONMENT=production
```

The app refuses to boot in production while these are still the localhost
defaults, because passkeys minted against `localhost` are ones no real browser
would ever send back — every registration would appear to succeed and every
later sign-in would fail.

**Passkeys registered locally will not work in production.** The RP ID is part
of the credential. That is correct behaviour, not a migration problem: register
again on the real host.
