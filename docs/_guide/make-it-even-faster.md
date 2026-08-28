---
title: Make It Even Faster
description: "Use a personal Spotify app for a separate quota while shared coverage stays active."
nav_order: 4
---

## The bottleneck is API rate limits

Everything Fastpotify shows you comes from Spotify's Web API, and Spotify
rate-limits that API per *app*: each app may make only so many requests a
minute. Out of the box Fastpotify uses a public app it shares with other
open-source players, so at busy times its requests queue behind everyone
else's. That is the spinner in the top bar, and pages that take a while
to fill.

An app of your own gives supported requests a separate Development Mode
quota. Fastpotify cannot ship one for everyone, but making yours is free and
takes a few minutes.

## Shared coverage stays active

Spotify keeps a personal app in Development Mode, and since February 2026 that
mode omits Spotify-owned playlists and reads playlist items only for playlists
you own or collaborate on. Artist top tracks, related artists,
recommendations, and some catalog fields are unavailable too. Fastpotify uses
the shared app for the complete playlist library, playlist-bearing search,
external playlist metadata and items, and those unavailable operations. Your
app accelerates supported work without replacing shared coverage.

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

1. Open **Settings**, find **Make it even faster**, and paste the
   Client ID.
2. Click **Authorize**. Your browser opens Spotify's sign-in for your app.
   Fastpotify verifies that it belongs to the same Spotify account, then shows
   **Personal acceleration is ready**.

That is all. Playing music on this computer is unaffected. Select **Remove**
to delete only the personal grant; the shared session stays signed in.
