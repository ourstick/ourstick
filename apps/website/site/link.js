// The page a sharing link opens without the app. The part after "#" is the
// stick's key: the browser never sends it to us, and this page only copies
// it to the clipboard when asked. It is never sent, stored or shown.
(function () {
  var button = document.getElementById("copy");
  var said = document.getElementById("copied");
  if (!location.hash) {
    document.getElementById("have-it").hidden = true;
    document.getElementById("broken").hidden = false;
    return;
  }
  if (/iPhone|iPad|iPod|Android/.test(navigator.userAgent) ||
      (/Macintosh/.test(navigator.userAgent) && navigator.maxTouchPoints > 1)) {
    document.getElementById("phone").hidden = false;
  }
  button.addEventListener("click", function () {
    navigator.clipboard.writeText(location.href).then(
      function () { said.textContent = "Copied. Now open OurStick: it finds the link by itself."; },
      function () { said.textContent = "Could not copy. Copy the address from the address bar."; }
    );
  });
})();
