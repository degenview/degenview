# Source this: `. ./tools/retry.sh`, then `retry "what it does" command args...`.
#
# Runs the command until it succeeds, up to RETRY_ATTEMPTS times (default 8), sleeping a growing
# delay between attempts: RETRY_BASE_DELAY seconds times the attempt number (default 20), capped at
# RETRY_MAX_DELAY (default 120). Returns the failure after the last attempt, with a GitHub Actions
# error annotation. Used for the notarization steps, which fetch tickets from Apple and now and then
# lose the connection ("network connection was lost", exit 68) after the build was Accepted; the
# release workflow and the disk-image check share it so their retry behaviour cannot drift.
retry() {
    what=$1; shift
    attempt=1
    until "$@"; do
        [ "$attempt" -lt "${RETRY_ATTEMPTS:-8}" ] || { echo "::error::$what failed $attempt times"; return 1; }
        delay=$((attempt * ${RETRY_BASE_DELAY:-20}))
        [ "$delay" -le "${RETRY_MAX_DELAY:-120}" ] || delay=${RETRY_MAX_DELAY:-120}
        echo "$what attempt ${attempt} failed; retrying in ${delay}s"
        sleep "$delay"
        attempt=$((attempt + 1))
    done
}
