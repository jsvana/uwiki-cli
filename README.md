# uwiki-cli

A CLI to administer [uwiki](https://github.com/jsvana/uwiki) installations.

## Configuration

Configuration is done via `~/.config/uwiki-cli/config.toml`.

**Required:**
* `username`: `uwiki` username, used for `uwiki-cli login`
* `password`: `uwiki` password, used for `uwiki-cli login`
* `token`: Generated via `uwiki-cli login`, used for updating pages

**Optional**:
* `server_address`: IP and port of the `uwiki` server to connect to (defaults to `http://localhost:1181`)

## Usage

```
$ uwiki-cli login
$ uwiki-cli set-page some/page/slug
```

## License
[MIT](LICENSE.md)
