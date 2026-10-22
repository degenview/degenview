#!/bin/sh
# Checks that the Production CloudKit schema can hold what this build uploads.
#
# Production's schema cannot be changed by the app: a field that was only ever created in the
# Development environment must be deployed ("Deploy Schema Changes" in the CloudKit Dashboard)
# before a release uses it, or every upload of that field fails and the data stays on the Mac.
# Run this before any release that touches the sync record (native/swift/Coordinator.swift,
# `record[...]`), and after every deploy.
#
# Needs a CloudKit management token, saved once with `xcrun cktool save-token` (create it in the
# CloudKit Dashboard under Settings, Tokens & Keys). It only reads schemas.
set -eu

team="${DEGENVIEW_TEAM_ID:-8JR6HMQNDE}"
container="${DEGENVIEW_CLOUDKIT_CONTAINER:-iCloud.es.nico.degenview}"
record_type="DegenViewSyncRecord"
# The plain (queryable by CloudKit) fields the bridge writes; `payload` and `modifiedAtMs` are
# encrypted values and have no schema entry.
fields="schemaVersion entityType revision originDevice payloadSize"

dir=$(mktemp -d)
trap 'rm -rf "$dir"' EXIT

for environment in development production; do
    if ! xcrun cktool export-schema --team-id "$team" --container-id "$container" \
        --environment "$environment" --output-file "$dir/$environment.ckdb" 2>"$dir/error"; then
        cat "$dir/error" >&2
        echo "could not read the $environment schema (is a management token saved? see the top of this script)" >&2
        exit 2
    fi
done

status=0
if ! grep -q "RECORD TYPE $record_type" "$dir/production.ckdb"; then
    echo "FAIL: Production has no record type $record_type" >&2
    status=1
else
    for field in $fields; do
        if ! grep -q "\"\?$field\"\? " "$dir/production.ckdb"; then
            echo "FAIL: Production's $record_type has no field $field" >&2
            status=1
        fi
    done
fi

if diff -u "$dir/production.ckdb" "$dir/development.ckdb" >"$dir/diff"; then
    echo "Production and Development schemas are identical."
else
    echo "Production and Development schemas differ (production first):" >&2
    cat "$dir/diff" >&2
    echo "Deploy the Development schema to Production, then run this again." >&2
    status=1
fi

[ "$status" -eq 0 ] && echo "ok: Production can hold the sync record."
exit "$status"
