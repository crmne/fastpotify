---
title: Make It Even Faster
description: "Use a personal Spotify app for a separate API quota."
nav_order: 6
---

## API rate limits

Fastpotify loads library and catalogue data through Spotify's Web API, which
is rate-limited per *app*. By default, Fastpotify shares a public app with
several other open-source players. When that app reaches its limit, requests
are delayed and the top bar shows a spinner. If Web API work stays busy for
five seconds and no personal app is configured, Fastpotify reminds you to set
one up in Settings at most once every 24 hours.

A personal app gives supported requests a separate Development Mode quota.
Creating one is free and takes a few minutes.

## Shared coverage stays active

Spotify keeps a personal app in Development Mode, and since February 2026 that
mode omits Spotify-owned playlists and reads playlist items only for playlists
you own or collaborate on. Artist top tracks, related artists,
recommendations, and some catalog fields are unavailable too. Fastpotify uses
the shared app for the complete playlist library, playlist-bearing search,
external playlist metadata and items, and those unavailable operations. Your
app handles supported requests. The shared app handles the rest.

## Make a Spotify app

1. Open the [Spotify developer dashboard](https://developer.spotify.com/dashboard)
   and sign in with your Spotify account. Spotify asks that it be a
   Premium account.
2. Click **Create app**. Any name and description will do; nobody else
   sees them.
3. Under **Redirect URIs**, add exactly:

   ```
   http://127.0.0.1:8989/login
   ```

4. Tick **Web API**, accept the terms, and save.
5. The app's page shows its **Client ID**. Copy it.

![Settings, with a personal Spotify app in use](/assets/images/make-it-even-faster.png)

## Use it in Fastpotify

1. Open **Settings**, find **Personal Spotify app**, and paste the
   Client ID.
2. Click **Authorize**. Your browser opens Spotify's sign-in for your app.
   Fastpotify verifies that it belongs to the same Spotify account, then shows
   **Personal app ready**.

This does not affect local playback. Select **Remove** to delete the personal
grant. The shared app stays signed in.
