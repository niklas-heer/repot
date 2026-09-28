#!/bin/sh
# Build the isolated world the demo tapes record in: a temporary home whose
# repository tree has ordinary git@github.com:owner/name.git remotes, served
# from local bare repositories by a stand-in ssh command. Nothing here touches
# the real home directory, real checkouts or the network.
#
#   sh demo/setup.sh [directory]    # default: /tmp/repot-demo
#
# Afterwards `. <directory>/home/.bashrc` enters the demo shell.
set -eu

demo=${1:-/tmp/repot-demo}
case $demo in
  /tmp/* | "${TMPDIR:-/tmp}"*) ;;
  *) echo "refusing to use $demo outside a temporary directory" >&2; exit 1 ;;
esac
rm -rf "$demo"
mkdir -p "$demo/home/.config/repot" "$demo/bin" "$demo/remotes" "$demo/seed"
# Resolve symlinks such as macOS's /tmp so the prompt can abbreviate to ~.
demo=$(cd "$demo" && pwd -P)

export HOME="$demo/home"
export XDG_CONFIG_HOME="$HOME/.config" XDG_STATE_HOME="$HOME/.local/state"
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL="$HOME/.gitconfig"
unset GHQ_ROOT GIT_DIR GIT_WORK_TREE
# No gh: every GitHub question falls back to plain Git fetches, so the
# recording never reaches the network.
PATH="$demo/bin:$PATH"
printf '#!/bin/sh\nexit 1\n' > "$demo/bin/gh"
# Git runs `ssh [options] git@<host> "git-upload-pack 'owner/name.git'"`;
# this stand-in runs that service in $demo/remotes/<host> instead.
cat > "$demo/bin/ssh" <<EOF
#!/bin/sh
for argument; do host=\${service:-}; service=\$argument; done
cd "$demo/remotes/\${host#*@}" && exec sh -c "\$service"
EOF
chmod +x "$demo/bin/gh" "$demo/bin/ssh"

cat > "$GIT_CONFIG_GLOBAL" <<EOF
[user]
	name = Octo Cat
	email = octo@example.com
[init]
	defaultBranch = main
[advice]
	detachedHead = false
[core]
	sshCommand = $demo/bin/ssh
[url "git@github.com:"]
	insteadOf = https://github.com/
EOF

cat > "$XDG_CONFIG_HOME/repot/repos.toml" <<'EOF'
[settings]
owners = ["octo"]
EOF

now=$(date +%s)
# Commit in the current directory, backdated by the given number of hours.
commit() {
  stamp="$((now - $1 * 3600)) +0000"
  shift
  GIT_AUTHOR_DATE=$stamp GIT_COMMITTER_DATE=$stamp git commit -q "$@"
}
# Change a file and commit it, backdated by hours.
change() {
  mkdir -p "$(dirname "$2")"
  printf '%s\n' "$3" >> "$2"
  git add "$2"
  commit "$1" -m "$4"
}

# Create a bare remote for host/owner/name with some history.
remote() {
  seed="$demo/seed/$1"
  git init -q "$seed"
  cd "$seed"
  change 900 README.md "# ${1##*/}" "Initial commit"
  change 500 src/main.rs "fn main() {}" "Add the application skeleton"
  change 200 .github/workflows/ci.yml "on: push" "Run tests in CI"
  git init -q --bare "$demo/remotes/$1.git"
  git remote add origin "$demo/remotes/$1.git"
  git push -q -u origin main
}

clone() {
  host=${1%%/*}
  git clone -q "git@$host:${1#*/}.git" "$HOME/ghq/$1"
}

for repository in \
  github.com/octo/api github.com/octo/website github.com/octo/cli \
  github.com/octo/notes github.com/octo/blog github.com/acme-corp/payments \
  github.com/acme-corp/infra github.com/acme-corp/design-system \
  github.com/rustacean/tiny-http gitlab.com/work-team/backend; do
  remote "$repository"
  clone "$repository"
done
# Not cloned yet, for `repot clone`.
remote github.com/octo/dotfiles

# api and backend fall behind: teammates pushed while we were away.
cd "$demo/seed/github.com/octo/api"
change 30 src/routes.rs "// health" "Add a health check endpoint"
change 20 src/routes.rs "// auth" "Require tokens on admin routes"
change 6 Cargo.toml "tokio = \"1\"" "Update tokio"
git push -q
cd "$demo/seed/gitlab.com/work-team/backend"
change 10 src/jobs.rs "// retry" "Retry failed jobs with backoff"
git push -q
cd "$demo/seed/github.com/acme-corp/design-system"
change 4 tokens.json "{}" "Add the terracotta colour token"
change 3 tokens.json "{}" "Tune spacing scale"
git push -q

# website: our dark-mode branch was squash-merged on GitHub and deleted.
cd "$HOME/ghq/github.com/octo/website"
git switch -q -c feat/dark-mode
change 50 src/theme.css ":root { color-scheme: dark; }" "Add a dark theme"
git push -q -u origin feat/dark-mode 2>/dev/null
cd "$demo/seed/github.com/octo/website"
git fetch -q origin
git merge -q --squash origin/feat/dark-mode >/dev/null
commit 26 -m "Add dark mode (#42)"
git push -q origin main :feat/dark-mode

# cli has a finished commit that was never pushed.
cd "$HOME/ghq/github.com/octo/cli"
change 2 src/main.rs "// --json" "Add --json output"

# payments has uncommitted work and is behind.
cd "$demo/seed/github.com/acme-corp/payments"
change 12 src/refunds.rs "// partial" "Support partial refunds"
git push -q
cd "$HOME/ghq/github.com/acme-corp/payments"
printf '// TODO: idempotency keys\n' >> src/main.rs

# blog diverged: a local commit and a different remote one.
cd "$HOME/ghq/github.com/octo/blog"
change 8 posts/hello.md "Hello!" "Draft the first post"
cd "$demo/seed/github.com/octo/blog"
change 7 posts/about.md "About me" "Add an about page"
git push -q

# A stray prototype outside the tree, for scan and adopt.
mkdir -p "$HOME/Projects/prototype"
cd "$HOME/Projects/prototype"
git init -q
change 40 index.html "<h1>hi</h1>" "Sketch the landing page"

# A week of visits so the picker ranks by frecency: `<unix seconds>\t<path>`.
mkdir -p "$XDG_STATE_HOME/repot"
for visit in 150:acme-corp/payments 120:octo/website 96:octo/api 72:octo/api \
  48:octo/website 30:octo/api 20:acme-corp/payments 5:octo/api 2:octo/website; do
  printf '%s\t%s\n' "$((now - ${visit%%:*} * 3600))" "$HOME/ghq/github.com/${visit#*:}"
done > "$XDG_STATE_HOME/repot/visits"

cat > "$HOME/.bashrc" <<EOF
export HOME="$HOME" XDG_CONFIG_HOME="$XDG_CONFIG_HOME" XDG_STATE_HOME="$XDG_STATE_HOME"
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL="$GIT_CONFIG_GLOBAL"
export PATH="$PATH" EDITOR=true BASH_SILENCE_DEPRECATION_WARNING=1
unset GHQ_ROOT GIT_DIR GIT_WORK_TREE NO_COLOR PROMPT_COMMAND
eval "\$(repot shell-init bash)"
PS1='\[\e[1;38;5;71m\]\w\[\e[0m\] \[\e[38;5;166m\]❯\[\e[0m\] '
cd "\$HOME"
EOF
