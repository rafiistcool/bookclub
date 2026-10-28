/**
 * A short name for a signed-in browser from its User-Agent (`SessionOut.label`
 * of a cookie session): "Firefox", "Linux". Either part is null when the
 * string does not say; the caller then shows the User-Agent as it is.
 */
export function describeUserAgent(ua: string): { browser: string | null; os: string | null } {
  const browser =
    /\bEdgA?\//.test(ua) ? "Edge"
    : /\b(OPR|Opera)\//.test(ua) ? "Opera"
    : /\b(Firefox|FxiOS)\//.test(ua) ? "Firefox"
    : /\bSamsungBrowser\//.test(ua) ? "Samsung Internet"
    : /\b(HeadlessChrome|Chrome|CriOS|Chromium)\//.test(ua) ? "Chrome"
    : /\bVersion\/[\d.]+.*\bSafari\//.test(ua) ? "Safari"
    : null;
  const os =
    /\b(iPhone|iPad|iPod)\b/.test(ua) ? "iOS"
    : /\bAndroid\b/.test(ua) ? "Android"
    : /\bWindows\b/.test(ua) ? "Windows"
    : /\bCrOS\b/.test(ua) ? "ChromeOS"
    : /\bMac OS X\b|\bMacintosh\b/.test(ua) ? "macOS"
    : /\bLinux\b/.test(ua) ? "Linux"
    : null;
  return { browser, os };
}
