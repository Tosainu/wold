# wold

Small HTTP fromtend for sending Wake-on-LAN magic packet.

    USAGE:
        wold [OPTIONS]
    
    OPTIONS:
        -l <address>:<port>     start a server with a provided address (default: 127.0.0.1:3000)
        -b <address>:<port>     send magic packets to a provided address (default: 255.255.255.255:9)
    
        --help, -h              display this message and exit

## Example Usage

### curl

    $ curl --json '{"target":"xx:xx:xx:xx:xx:xx"}' <address>:<port>

### Shortcuts

![](img/shortcuts.png)
