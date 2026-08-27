<#--
  Hands the browser back to the application instead of Keycloak's two stock
  recovery links.

  Keycloak renders this page from AuthenticationFlowURLHelper.showPageExpired()
  — reached when a request carries an `execution` that no longer matches the
  authentication session's current one (a returning IdP callback whose
  tab-scoped session was torn down or rotated while several logins raced in one
  browser), and when a required-action session goes stale. Its "restart" link
  needs the browser-wide KC_RESTART cookie, which a completed sibling login
  deletes, and its "continue" link points at the execution that already failed
  to match. When both hold there is no way off the page.

  This is NOT the page Keycloak renders when it rebuilds a login from KC_RESTART
  and stamps `loginTimeout` on it. That path ends in
  AuthenticationProcessor.authenticateOnly(), which calls createErrorPage() and
  serves `error.ftl`. That page is left stock on purpose: its message already
  explains the timeout, and the client's `baseUrl` in the realm gives it a
  working way out. Overriding it would auto-redirect every Keycloak error,
  masking the ones a user needs to read.

  A fresh visit to the application always succeeds, so that is where this goes.
  The `auth_error` marker in the target drives the SPA's existing
  retry-once-then-stop guard, so a persistent failure stops on the login error
  screen instead of looping browser -> IdP.
-->
<#assign returnUrl = (properties.insightReturnUrl)!"/?auth_error=kc_page_expired">
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <meta name="robots" content="noindex, nofollow">
  <meta http-equiv="refresh" content="0; url=${returnUrl}">
  <title>Signing you back in</title>
  <style>
    body {
      margin: 0;
      min-height: 100vh;
      display: flex;
      align-items: center;
      justify-content: center;
      font-family: system-ui, -apple-system, "Segoe UI", Roboto, sans-serif;
      font-size: 15px;
      line-height: 1.5;
      color: #1b1b1f;
      background: #f4f4f6;
    }
    @media (prefers-color-scheme: dark) {
      body { color: #e8e8ec; background: #16161a; }
    }
    main { text-align: center; padding: 24px; }
    a { color: inherit; }
  </style>
</head>
<body>
  <main>
    <p>That sign-in took too long. Taking you back to Insight&hellip;</p>
    <p><a id="continue" href="${returnUrl}">Continue</a></p>
  </main>
  <#-- Read the target off the anchor rather than interpolating it into JS:
       one escaping context instead of two, and `replace` keeps this page out
       of history so Back cannot land on it again. -->
  <script>
    var link = document.getElementById("continue");
    if (link) { window.location.replace(link.href); }
  </script>
</body>
</html>
