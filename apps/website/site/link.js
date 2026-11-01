// The page a link opens without the app, https://ourstick.com/s/#<public>.<secret>.
// All of it after "#" (the key, and who and where the sender is): the
// browser never sends it to us, and this page only copies it to the
// clipboard when asked. It is never sent, stored or shown. "Open in
// OurStick" also opens ourstick:receive, which carries nothing: OurStick
// comes to the front and finds the copied link itself (ADR 0030).
(function () {
  var button = document.getElementById("copy");
  var said = document.getElementById("copied");
  // Punctuation a chat app left at its end is not part of it (the app drops it).
  if (!/^#[a-z2-7]+\.[a-z2-7]+[.,;:!?)\]}>"'`]*$/i.test(location.hash)) {
    document.getElementById("have-it").hidden = true;
    document.getElementById("broken").hidden = false;
    return;
  }
  // This computer's download first (downloads.js), the others under one
  // link: "Mac or Linux".
  var list = document.querySelector(".downloads");
  var mine = list && list.querySelector(".download.mine");
  if (mine) {
    var others = Array.prototype.filter.call(list.querySelectorAll(".download"), function (item) {
      return item !== mine;
    });
    var more = document.createElement("details");
    var summary = document.createElement("summary");
    summary.textContent = others.map(function (item) { return item.dataset.name; }).join(" or ");
    more.appendChild(summary);
    var inside = document.createElement("div");
    inside.className = "downloads";
    others.forEach(function (item) { inside.appendChild(item); });
    more.appendChild(inside);
    list.parentNode.insertBefore(more, list.nextSibling);
  }
  button.addEventListener("click", function () {
    // Both in the click itself: a browser opens an app only right after
    // one. The copy is done when asked; OurStick reads it once in front.
    var copying = navigator.clipboard.writeText(location.href);
    openApp();
    copying.then(
      function () {
        button.textContent = "Copied ✓";
        said.textContent = "Nothing opened? Get OurStick above, open it and press Receive. The link is already copied.";
      },
      function () { said.textContent = "Could not copy. Copy the address from the address bar."; }
    );
  });
  // From a hidden frame, so that this page stays where nothing opens it.
  function openApp() {
    var old = document.getElementById("open-app");
    if (old) old.remove();
    var frame = document.createElement("iframe");
    frame.id = "open-app";
    frame.hidden = true;
    frame.src = "ourstick:receive";
    document.body.appendChild(frame);
  }
})();
