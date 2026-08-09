# Security policy

## What this application is, and why that shapes the threat model

Open Takeout Hub reads the archive a person downloads from Google because they
want their data back. That archive holds their photographs, their contacts,
their calendar, their documents: often the most complete record of a life that
exists in one place.

The application makes one promise, and the whole design rests on it:

> The application process opens no network connections.

Everything else follows from that. How the promise is made executable, and how
to verify it yourself, is documented in [PRIVACY_AUDIT.md](PRIVACY_AUDIT.md).

## What counts as a vulnerability here

Anything that breaks one of these, whether or not it needs a malicious file to
trigger:

- **Data leaving the machine.** Any network connection, DNS lookup or telemetry,
  from the Rust process or from the webview.
- **Writing outside the chosen paths.** The application writes only where the
  user pointed it. An archive entry escaping the destination (zip-slip) belongs
  here, and so does anything writing outside the folder a dialog returned.
- **Losing data.** No function deletes files. A path that destroys, truncates or
  overwrites something the user did not agree to lose is a vulnerability, not a
  bug: an export is often the only remaining copy.
- **Command or argument injection.** The one place the application reaches the
  operating system is the "Show in file manager" button, described in section
  7b of the audit. Anything that turns a filename into an executed command
  belongs here.
- **Reading a crafted file causing memory unsafety.** The parsers handle
  untrusted input: ZIP entries, EXIF blocks, vCard, iCalendar and JSON sidecars
  all come from a file the user did not write.

Crashing on a malformed file is a bug, and worth reporting as an issue, but it
is not a vulnerability on its own: the application processes files, so a bad
file should produce an error rather than a panic.

## Out of scope

- The system webview itself (WKWebView, WebKit2GTK, WebView2). It is an
  operating system component: report those to its vendor. What is in scope is
  our confinement of it, that is the Content Security Policy in
  `src-tauri/tauri.conf.json`.
- Anything requiring an attacker who already runs code as the user. At that
  point the export is readable without going through this application.
- The absence of code signing. It is a known gap, recorded in the changelog,
  not a finding.

## How to report

**Please do not open a public issue for a security problem.** A public report
tells everyone how to exploit it before there is a fix.

Use GitHub's private reporting: the **Security** tab of this repository, then
**Report a vulnerability**. It opens a channel visible only to the maintainer.

Useful in a report, roughly in order of usefulness:

- what an attacker gains, in one sentence
- the steps to reproduce, and a sample file if the trigger is a crafted one
- the version, from the guide, and the operating system
- whether it needs the user to do something, and what

A file that reproduces the problem is worth more than a long description. If it
holds real personal data, describe how to build an equivalent one rather than
sending it: this project exists to keep such files off other people's machines,
and that includes the maintainer's.

## What happens then

This is maintained by one person, so no response time is promised that could
not be kept. What is promised instead:

- a report is acknowledged when it is read, even if the answer is that it needs
  time
- a confirmed finding is fixed before anything else
- the fix says what was wrong and since which version, in the changelog
- credit goes to whoever reported it, unless they prefer otherwise

## Supported versions

Nothing has been released yet. When it is, only the latest version is
supported: there is no back-porting to older ones.
