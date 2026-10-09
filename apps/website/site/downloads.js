// Puts the visitor's own system first. Reads nothing but the user agent;
// sends nothing anywhere.
(function () {
  // A phone (an iPad says "Macintosh" but has touch) gets none of them first.
  var ua = navigator.userAgent;
  var phone = /iPhone|iPad|iPod|Android/.test(ua) ||
    (/Macintosh/.test(ua) && navigator.maxTouchPoints > 1);
  var mine = phone ? null : /Windows/.test(ua) ? "windows" : /Macintosh/.test(ua) ? "macos"
    : /Linux/.test(ua) && !/CrOS/.test(ua) ? "linux" : null;
  var list = document.querySelector(".downloads");
  var item = mine && list && list.querySelector('[data-os="' + mine + '"]');
  if (item) {
    item.classList.add("mine");
    list.insertBefore(item, list.firstChild);
  }
})();
