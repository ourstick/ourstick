// The page a link opens without the app, https://ourstick.com/s/#<public>.<secret>.
// All of it after "#" (the key, and who and where the sender is): the
// browser never sends it to us, and this page only copies it to the
// clipboard when asked. It is never sent, stored or shown.
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
    navigator.clipboard.writeText(location.href).then(
      function () {
        button.textContent = "Copied ✓";
        said.textContent = "Now open OurStick and press Receive.";
      },
      function () { said.textContent = "Could not copy. Copy the address from the address bar."; }
    );
  });
})();
