# Studio Hub

A launcher and auto updater for Studio Robi VST3 plugins on Windows.

One app to browse, install, update, and remove plugins, with no separate
installer for each one.

## Features

**Explore**
Browse every available plugin with its logo, category, and a short summary.
Opening a plugin shows the full README from its repository along with a list
of versions, so older releases stay available.

**Library**
Shows the plugins installed on your machine, their versions, when they were
installed, and whether their files are still intact. Only Studio Robi plugins
appear here, so plugins from other vendors are left alone.

**Updates**
Tells you which plugins have a newer release, with the changelog visible up
front. Non breaking updates can be installed in one go, while updates that
could break an existing project are always confirmed one at a time. Individual
versions can be skipped.

**Installs that fail safely**
Every download is verified with SHA-256 before it is touched, and there is no
way to bypass that. Installation is atomic: the old version is set aside, the
new one is put in place, and if anything fails partway through, the previous
install is left intact.

**Rollback**
The previous version is always kept, so you can go back at any time without
downloading anything again.

**No Administrator needed**
By default plugins install to a per user folder, so there is no UAC prompt at
all. Installing to a system wide location asks for elevation only when it is
actually required.

**DAW detection**
Refuses to overwrite a plugin that an open DAW is currently using, instead of
breaking a running session.

**Finds plugins you already have**
Studio Robi plugins that were previously installed by hand are detected and
brought under management, with no need to reinstall them.

**Self updating**
Studio Hub updates itself, and accepts only updates whose signature matches.

**Interface**
Dark and light themes, English and Indonesian, a collapsible sidebar, and
automatic display scaling.

## System requirements

Windows 10 version 1809 or later, 64 bit.

## License

Proprietary. All rights reserved. The source is viewable for transparency and
auditing, but redistributing the source or the application is not permitted.
See [LICENSE](LICENSE).
