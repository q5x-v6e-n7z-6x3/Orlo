# Security

## Reporting a vulnerability

Please report security issues privately, through GitHub's private vulnerability reporting:
open the repository's **Security** tab and choose **Report a vulnerability**. Do not open a
public issue for them. Include the Orlo commit you tested, the steps or files that reproduce the
problem, and what an attacker gains.

Reports are acknowledged as soon as they are read; fixes land on `main` with a regression test,
and the advisory is published once a fix is available.

## What Orlo treats as untrusted

- **Files and folders you open**, including a repository's own `.git/config`, `.gitattributes`,
  `.editorconfig`, modelines, project and workspace files, and tasks. A folder's own programs
  (Git filter drivers, language servers that build the project, project tasks) run only after
  you trust it; see [Trusted folders](README.md#trusted-folders).
- **Output printed in the integrated terminal**: escape sequences cannot set the clipboard,
  answer with attacker text, or run commands, and only `http(s)` links open in a browser.
- **Other local users**: the single-instance socket and the support folder are private to you.

The language servers, formatters and shell commands you configure yourself run with your
permissions, as they would in a terminal.
