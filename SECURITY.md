# Security

wapit is a development tool meant to run on your own machine. It has no authentication: it binds to `127.0.0.1` and refuses requests whose `Host` is not a local name. Exposing its ports to a network is outside its intended use.

## Reporting a vulnerability

Report it privately through GitHub: open the repository's **Security** tab and choose **Report a vulnerability**. Include the version, how you run wapit, and the steps to reproduce.

Issues in scope include anything that lets a web page, another user on the network, or a crafted payload read the inbox, run code in it, reach files outside wapit's data and config directories, or make wapit send requests the user did not cause.
