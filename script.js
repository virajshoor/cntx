const terminalLines = [
  "$ cntx --apply --mode allow \"add a checklist to the docs\"",
  "-> endpoint=work model=claude-sonnet-4.5 mode=apply tokens=842 saved=310 chars",
  "... working",
  "",
  "**Plan**",
  "- update README",
  "- write docs/apply.md",
  "- keep writes inside the sandbox",
  "",
  "file checklist",
  "  [written] README.md write within sandbox",
  "  [written] docs/apply.md write within sandbox",
  "",
  "$ /checklist",
  "file checklist",
  "  [written] README.md write within sandbox",
  "  [written] docs/apply.md write within sandbox"
];

const demo = document.querySelector("#terminal-demo code");
let index = 0;

function drawTerminal() {
  if (!demo) return;
  demo.textContent = terminalLines.slice(0, index).join("\n");
  index += 1;
  if (index > terminalLines.length) {
    window.setTimeout(() => {
      index = 0;
      drawTerminal();
    }, 1600);
    return;
  }
  window.setTimeout(drawTerminal, index === 3 ? 600 : 120);
}

drawTerminal();

document.querySelectorAll("[data-copy]").forEach((button) => {
  button.addEventListener("click", async () => {
    const value = button.getAttribute("data-copy");
    try {
      await navigator.clipboard.writeText(value);
      const original = button.textContent;
      button.textContent = "copied";
      button.classList.add("copied");
      window.setTimeout(() => {
        button.textContent = original;
        button.classList.remove("copied");
      }, 1100);
    } catch {
      button.textContent = value;
    }
  });
});
