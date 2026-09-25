# Professional Nouner

So I'd only been doing serious projects and really wanted to take a break and
make something silly. I don't remember the inspiration for it, but I decided
that exactly the project I needed to make was a bot that allows you to set a
list of pronouns, of which one set is chosen at random each day (resetting at
midnight UTC). Optionally the bot can send an update with the new pronouns
for all registered users in a server, and it may also on a per-user basis add
that user's pronouns as a suffix to their server nickname.

# Using this bot

I'm hosting this bot off my own personal computer, and if you trust that enough
to use it then you can use [this][install] link to invite it to any servers
that you have the permissions to do so in. Otherwise...

# Configuring this bot for use yourself

After creating a bot in the Discord developer portal, you will need to give it
the following permissions:

  - Standard installation settings (under `Installation` tab):
    - User installation scopes:
      - `applications.commands`
    - Guild installation:
      - Scopes:
        - `applications.command`
        - `bot`
      - Permissions:
        - Send messages
        - Send messages in threads
        - Embedded links
        - Change nickname
        - Manage nicknames
        - Use slash commands
  - Privileged Gateway Intents (under `Bot` tab):
    - Presence Intent
    - Server Members Intent

Then to run this bot, clone this repository, build it with `cargo build` (or
optionally `cargo build --release`), create an empty `data.toml` file, and then
run the binary with the `DISCORD_TOKEN` environment variable set to your bot's
API token. Personally I placed this in `.cargo/config.toml` with the following:
```toml
[env]
DISCORD_TOKEN = "..."
```
But you can do this however you like. The bot should handle on its own adding
in new guilds to its data file as it is invited and removed from servers, and
it should be able to add and prune members as they register/deregister. Though
if it doesn't work as intended, please file an issue to let me know!

# Limitations and permissions:

There's two really annoying things that there's no way to work around.

  1. The bot cannot change the server owner's nickname, ever. It's just not
     possible.
  2. The bot can only change the nicknames of users whose most privileged role
     is lower in the heirarchy than the bot's role (or its most privileged
     role? I forget).

There's no way to resolve point 1, but to fix point 2 you can just put the bot
at the top of the roles list, or above the highest role possessed by anyone who
wishes to have their nickname changed.

# Notes about pronouns

I'm limited to only 80 (or 120? I forget) characters in the slash command help
messages, so here's some wordier clarification on what you are and aren't
allowed to do in your user's pronoun settings. 

  - You may have any number of sets of pronouns, limited only by the number of
    characters that fit in a slash command argument. I actually don't know what
    this limit is off the top of my head, and a cursory search doesn't turn up
    a useful answer. I think it might be 4000 characters?
  - Each pronoun "set" (delimited by commas) may only be 10 characters or
    fewer. This is an arbitrary number I made up out of nowhere and have no
    memory of why I chose this.
  - Pronoun sets may only contain alphabetic characters and `/`. This means
    characters like `3` are not allowed, but others like `ĳ` are.

When you enter these into the bot with the `/settings pronouns_set_local` or
`/settings pronouns_set_global` commands, make sure that you do not use a space
when separating the sets! `he/him,they/them` is valid, but `he/him, they/them`
is not!

## Neat little hack

Because this was meant to be a very low-effort project to unwind (it didn't end
up being one, unfortunately), I didn't include a way for you to adjust the
probability distribution of the pronouns, even though I kind of wanted to. As
such a uniform distribution is used to sample the available list of pronouns.
But there's a way you can kind of get around this, which is just to include the
pronouns you want a higher chance of rolling more often. Let's say for instance
that you want a 60% chance of `they/them` and 40% chance of `it/its`. For this,
you can use `they/them,they/them,they/them,it/its,it/its` as your pronouns
list. 3/5 are `they/them` (60%), and 2/5 are `it/its` (40%). You can probably
approximate whatever distribution is interesting to you using this method.

# Bot announcements (`/announcement`)

This command has an optional parameter, `channel`, which will tell the bot
which channel to send a message to when user pronouns are randomised. If this
parameter is not provided, then the bot will not attempt to send an
announcement with updated pronouns. In short:

  - To enable announcements, do `/announcement channel:#...`
  - To disable announcements, do `/announcement`

# License

This code is licensed under the European Union Public License version 1.2.

# Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, the contributor (as defined in the EUPL
license), shall be licensed as above without any additional terms or conditions.

[install]: https://discord.com/oauth2/authorize?client_id=1526934245296050227
