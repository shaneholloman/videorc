# Live Chat — Real Provider Smoke Checklist

Slice 10 of the In-App Livestream Comments plan. These are **manual, gated** smokes: they
need live platform accounts and a real broadcast, so they are not part of the automated
suite. The automated, account-free path is `pnpm smoke:live-chat-fake-providers`, which
drives the coordinator + fake connector end to end over the real websocket protocol.

## Automated (CI-able)

- [ ] `pnpm smoke:oauth-guards` — OAuth guard behavior.
- [ ] `pnpm smoke:provider-readiness` — per-provider live-smoke prerequisites (secrets present).
- [ ] `pnpm smoke:live-chat-fake-providers` — start → messages → de-dupe → diagnostics →
      clear → stop, plus the capability surface and the X capability gate. No OAuth required.

## Capture-performance regression

- [ ] Run `pnpm smoke:recording-performance` (and/or `pnpm smoke:preview-performance`) once
      with a fake live-chat session active (`liveChat.start` with a fake config) and confirm
      preview/recording metrics stay within their existing tolerances. Chat networking runs on
      isolated spawned async tasks that only touch provider status + the bounded buffer, never
      the capture/encode path, so there should be no regression.

## YouTube live chat (deferred until Google approval)

- [ ] Confirm YouTube chat readiness reports the Google approval pause message.
- [ ] Do not connect a YouTube OAuth account or run YouTube chat acceptance until Google approval completes.
- [ ] Use Manual RTMP for YouTube stream acceptance in the meantime.

## Twitch OAuth live smoke (requires a Twitch account, reconnected for `user:read:chat`)

- [ ] Reconnect Twitch so the granted scopes include `user:read:chat` (preflight chat readiness
      should flip to ready).
- [ ] Go Live to Twitch; confirm the panel shows Twitch `connected` (EventSub welcome →
      subscriptions created).
- [ ] Post chat from another account incl. an emote + a cheer; confirm fragments + badges +
      the bits amount render, and duplicate EventSub deliveries are not double-shown.
- [ ] Force a reconnect (toggle network); confirm the provider shows `reconnecting` then
      recovers, and `liveChat.diagnostics` reconnect count increments.

## X (gated — do NOT run until a verified native path exists)

- [ ] Only run after `X_NATIVE_COMMENTS_AVAILABLE` is intentionally enabled behind a verified,
      approved native X comments API (see `x_chat.rs` evidence checklist). Until then X must
      show `unsupported` / "X comments pending API access" and the feature is **not** "done".

## Multistream + partial release

- [ ] Go Live to YouTube Manual RTMP + Twitch + X simultaneously; confirm a single unified panel shows
      every available platform's state, Twitch comments chronologically,
      YouTube as paused for Google approval, and X as `pending API access`
      (comments blocked) without blocking Go Live.
- [ ] Confirm the streamer can read all comments from the in-app panel **without opening any
      platform dashboard**.
