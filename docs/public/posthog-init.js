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

  // Chromium often strips utm_* after the arete.run /x/<slug> hop.
  // arete_attr on .arete.run is the fallback for docs PostHog.
  /* arete-attr:start */
  const ARETE_ATTR_COOKIE = "arete_attr";
  const ARETE_ATTR_UTM_KEYS = [
    "utm_source",
    "utm_medium",
    "utm_campaign",
    "utm_content",
    "utm_term",
  ];
  // Decoded attribution values are short tokens (week ids, slug_account, "+").
  const ARETE_ATTR_TOKEN = /^[A-Za-z0-9][A-Za-z0-9._+\-]{0,127}$/;

  function readAreteAttr(cookieHeader) {
    if (!cookieHeader) return null;
    const prefix = `${ARETE_ATTR_COOKIE}=`;
    let raw = null;
    for (const part of cookieHeader.split(/;\s*/)) {
      if (part.startsWith(prefix)) {
        raw = part.slice(prefix.length);
        break;
      }
    }
    if (!raw) return null;

    // The cookie may be a raw query string or one percent-encoded value.
    // decodeURIComponent keeps "+", which URLSearchParams would turn into a space.
    let body = raw;
    if (!body.includes("&") && body.includes("%")) {
      try {
        body = decodeURIComponent(body);
      } catch {
        return null;
      }
    }
    if (!body.includes("=")) return null;

    const attr = {};
    for (const pair of body.split("&")) {
      const eq = pair.indexOf("=");
      if (eq <= 0) return null;
      const key = pair.slice(0, eq);
      if (!ARETE_ATTR_UTM_KEYS.includes(key)) return null;
      if (Object.prototype.hasOwnProperty.call(attr, key)) return null;
      let value = pair.slice(eq + 1);
      try {
        value = decodeURIComponent(value.replace(/\+/g, "%2B"));
      } catch {
        return null;
      }
      if (!ARETE_ATTR_TOKEN.test(value)) return null;
      attr[key] = value;
    }

    if (attr.utm_source !== "x" || attr.utm_medium !== "social") {
      return null;
    }
    return attr;
  }

  function queryUtms(search) {
    const params = new URLSearchParams(
      search.charAt(0) === "?" ? search.slice(1) : search,
    );
    const utm = {};
    for (const key of ARETE_ATTR_UTM_KEYS) {
      const value = params.get(key);
      if (value) utm[key] = value;
    }
    return utm;
  }

  // A page URL is a newer X campaign only when it carries the same shape the
  // cookie reader accepts. Other query UTMs must not replace arete_attr.
  function xAttrFromQuery(utm) {
    if (utm.utm_source !== "x" || utm.utm_medium !== "social") return null;
    const attr = {};
    for (const key of ARETE_ATTR_UTM_KEYS) {
      if (!utm[key]) continue;
      if (!ARETE_ATTR_TOKEN.test(utm[key])) return null;
      attr[key] = utm[key];
    }
    return attr;
  }

  function writeAreteAttr(attr) {
    if (typeof document === "undefined") return;
    const body = ARETE_ATTR_UTM_KEYS.filter((key) => attr[key])
      .map((key) => `${key}=${encodeURIComponent(attr[key])}`)
      .join("&");
    document.cookie = `${ARETE_ATTR_COOKIE}=${body}; Domain=.arete.run; Path=/; Max-Age=1800; SameSite=Lax; Secure`;
  }

  function applyAreteAttr(event, cookieHeader, search) {
    if (!event || event.event === "$snapshot") return event;
    const header =
      cookieHeader == null
        ? typeof document === "undefined"
          ? ""
          : document.cookie
        : cookieHeader;
    const attr = readAreteAttr(header);

    // Only utm_* on this page's URL are current query attribution.
    // posthog.register copies earlier visits onto every event, so a filled
    // property is not proof the query string is still here.
    const queryString =
      search == null
        ? typeof location === "undefined"
          ? ""
          : location.search
        : search;
    const fromUrl = queryUtms(queryString);
    // The query campaign is newer than a cookie from an earlier visit.
    // Refresh arete_attr so the next docs page, which has no query, keeps it.
    const fresh = xAttrFromQuery(fromUrl);
    if (fresh) writeAreteAttr(fresh);
    if (!attr) return event;

    const props = event.properties || (event.properties = {});
    for (const key of ARETE_ATTR_UTM_KEYS) {
      if (fromUrl[key]) {
        props[key] = fromUrl[key];
        continue;
      }
      if (attr[key]) props[key] = attr[key];
    }

    if (event.event === "$pageview") {
      const set = props.$set || (props.$set = {});
      const setOnce = props.$set_once || (props.$set_once = {});
      for (const key of ARETE_ATTR_UTM_KEYS) {
        const value = props[key];
        if (!value) continue;
        // Cookie-replaced keys and URL keys both follow the winner above.
        set[key] = value;
        setOnce[`$initial_${key}`] = value;
      }
    }
    return event;
  }
  /* arete-attr:end */

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
    before_send: applyAreteAttr,
  });

  // Query UTMs that survived (Firefox) stay on later docs events.
  // A later arete_attr cookie replaces a saved value when this page's
  // URL has no utm_* for that key.
  // `register` is on the stub, so this queues until array.js finishes loading.
  const utm = {};
  const params = new URLSearchParams(window.location.search);
  [
    "utm_source",
    "utm_medium",
    "utm_campaign",
    "utm_content",
    "utm_term",
  ].forEach((key) => {
    const value = params.get(key);
    if (value) utm[key] = value;
  });
  if (Object.keys(utm).length > 0) {
    window.posthog.register(utm);
  }
})();
