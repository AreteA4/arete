// PostHog initialization — externalized from inline <script> to keep the
// document's pre-content bytes small. AFDocs (https://afdocs.dev) measures
// where main content begins in the converted markdown of each page; an
// inline ~3KB PostHog snippet pushes content past the 50% mark and fails the
// "Content Start Position" check. Loading this with `defer` keeps analytics
// behavior identical (init runs after parse, before DOMContentLoaded).
//
// API key is read from the data-posthog-key attribute on the loading <script>.

(function () {
  if (window.__posthog_initialized) return;
  // Set the flag before any further work — including the missing-key path —
  // so re-executions (Astro view transitions, devtools reloads) don't repeat
  // the warning or re-run init. Matches the original inline script behavior.
  window.__posthog_initialized = true;

  const scriptEl =
    document.currentScript ||
    document.querySelector("script[data-posthog-key]");
  const posthogKey = scriptEl?.getAttribute("data-posthog-key");

  if (!posthogKey) {
    console.warn(
      "[PostHog] No API key found. Set PUBLIC_POSTHOG_KEY environment variable.",
    );
    return;
  }

  // Standard PostHog snippet — keep verbatim, do not reformat.
  !(function (t, e) {
    var o, n, p, r;
    e.__SV ||
      ((window.posthog = e),
      (e._i = []),
      (e.init = function (i, s, a) {
        function g(t, e) {
          var o = e.split(".");
          (2 == o.length && ((t = t[o[0]]), (e = o[1])),
            (t[e] = function () {
              t.push([e].concat(Array.prototype.slice.call(arguments, 0)));
            }));
        }
        (((p = t.createElement("script")).type = "text/javascript"),
          (p.crossOrigin = "anonymous"),
          (p.async = !0),
          (p.src = s.api_host + "/static/array.js"),
          (r = t.getElementsByTagName("script")[0]).parentNode.insertBefore(
            p,
            r,
          ));
        var u = e;
        for (
          void 0 !== a ? (u = e[a] = []) : (a = "posthog"),
            u.people = u.people || [],
            u.toString = function (t) {
              var e = "posthog";
              return (
                "posthog" !== a && (e += "." + a),
                t || (e += " (stub)"),
                e
              );
            },
            u.people.toString = function () {
              return u.toString(1) + ".people (stub)";
            },
            o =
              "capture identify alias people.set people.set_once set_config register register_once unregister opt_out_capturing has_opted_out_capturing opt_in_capturing reset isFeatureEnabled onFeatureFlags getFeatureFlag getFeatureFlagPayload reloadFeatureFlags group updateEarlyAccessFeatureEnrollment getEarlyAccessFeatures getActiveMatchingSurveys getSurveys getNextSurveyStep onSessionId".split(
                " ",
              ),
            n = 0;
          n < o.length;
          n++
        )
          g(u, o[n]);
        e._i.push([i, s, a]);
      }),
      (e.__SV = 1));
  })(document, window.posthog || []);

  // Use reverse proxy on Vercel to bypass ad-blockers; direct EU URL for local dev.
  const isLocalhost =
    window.location.hostname === "localhost" ||
    window.location.hostname === "127.0.0.1";
  const apiHost = isLocalhost ? "https://eu.i.posthog.com" : "/ingest";

  window.posthog.init(posthogKey, {
    api_host: apiHost,
    ui_host: "https://eu.posthog.com",
    person_profiles: "always",
    capture_pageview: true,
    capture_pageleave: true,
    autocapture: true,
    session_recording: {
      maskAllInputs: false,
      maskInputOptions: { password: true },
    },
  });
})();
