// The page a sharing link opens without the app,
// https://ourstick.com/s/#<public>.<secret>. All of it after "#" (the stick's
// key, and who and where the sender is): the browser never sends it to us,
// and this page only copies it to the clipboard when asked. It is never
// sent, stored or shown.
(function () {
  var button = document.getElementById("copy");
  var said = document.getElementById("copied");
  // Punctuation a chat app left at its end is not part of it (the app drops it).
  if (!/^#[a-z2-7]+\.[a-z2-7]+[.,;:!?)\]}>"'`]*$/i.test(location.hash)) {
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
      function () { said.textContent = "Copied. Now open the app: it finds the link by itself."; },
      function () { said.textContent = "Could not copy. Copy the address from the address bar."; }
    );
  });
})();
