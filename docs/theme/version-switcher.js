// Version picker for the docs site. Each version is published to its own
// directory on GitHub Pages (`main/`, `v0.1.0/`, ...), next to a
// `versions.json` written by .github/workflows/docs.yml.
(function () {
  // mdBook defines `path_to_root` on every page.
  if (typeof path_to_root === "undefined") return;
  var bookRoot = new URL(path_to_root || "./", window.location.href);
  var current = bookRoot.pathname.replace(/\/$/, "").split("/").pop();
  var siteRoot = new URL("../", bookRoot);

  fetch(new URL("versions.json", siteRoot))
    .then(function (r) { return r.ok ? r.json() : []; })
    .then(function (versions) {
      if (!versions.length) return;
      var select = document.createElement("select");
      select.setAttribute("aria-label", "Documentation version");
      versions.forEach(function (v) {
        var option = document.createElement("option");
        option.value = v;
        option.textContent = v === "main" ? "main (unreleased)" : v;
        option.selected = v === current;
        select.appendChild(option);
      });
      select.addEventListener("change", function () {
        var page = window.location.pathname.slice(bookRoot.pathname.length);
        window.location.href = new URL(select.value + "/" + page, siteRoot).href;
      });
      var bar = document.querySelector(".right-buttons");
      if (bar) bar.prepend(select);
    })
    .catch(function () {});
})();
