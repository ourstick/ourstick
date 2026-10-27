// Plain old JavaScript on purpose: it runs where the app cannot. The app
// needs a web view as new as Chrome 88 or Safari 14 (ADR 0027); in an older
// one it would stay blank, so this says what to do instead.
(function () {
  var css = window.CSS;
  var ok =
    typeof "".replaceAll === "function" &&
    !!css &&
    typeof css.supports === "function" &&
    css.supports("selector(:where(a))");
  if (ok) return;
  document.addEventListener("DOMContentLoaded", function () {
    var note = document.createElement("p");
    note.style.cssText = "font: 16px/1.5 sans-serif; margin: 0; padding: 24px; color: #222; background: #fff";
    note.textContent =
      "OurStick needs a newer Android System WebView. Update it in Google Play " +
      "(look for “Android System WebView”), then open OurStick again.";
    document.body.innerHTML = "";
    document.body.appendChild(note);
  });
})();
