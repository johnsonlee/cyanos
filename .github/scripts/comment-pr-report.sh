#!/usr/bin/env bash
set -euo pipefail

: "${GH_TOKEN:?GH_TOKEN is required}"
: "${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is required}"
: "${GITHUB_RUN_ID:?GITHUB_RUN_ID is required}"
: "${GITHUB_SERVER_URL:?GITHUB_SERVER_URL is required}"

test_report="${TEST_REPORT_PATH:-test-report.txt}"
coverage_report="${COVERAGE_REPORT_PATH:-coverage-report.txt}"
marker="<!-- cyanos-ci-report -->"

resolve_pr_number() {
  if [[ "${GITHUB_EVENT_NAME:-}" == "pull_request" ]]; then
    jq -r '.pull_request.number' "${GITHUB_EVENT_PATH:?GITHUB_EVENT_PATH is required}"
    return
  fi

  gh pr view "${GITHUB_REF_NAME:?GITHUB_REF_NAME is required}" \
    --repo "${GITHUB_REPOSITORY}" \
    --json number \
    --jq '.number'
}

resolve_commit_sha() {
  if [[ "${GITHUB_EVENT_NAME:-}" == "pull_request" ]]; then
    jq -r '.pull_request.head.sha' "${GITHUB_EVENT_PATH:?GITHUB_EVENT_PATH is required}"
    return
  fi

  echo "${GITHUB_SHA:-unknown}"
}

print_clean_report_tail() {
  local path="$1"
  local lines="${2:-120}"

  if [[ -f "${path}" ]]; then
    PERL_BADLANG=0 perl -pe 's/\e\[[0-9;?]*[ -\/]*[@-~]//g' "${path}" | tail -n "${lines}"
  else
    echo "Report was not generated."
  fi
}

test_status() {
  local path="$1"

  if [[ ! -f "${path}" ]]; then
    echo "Not available"
    return
  fi

  PERL_BADLANG=0 perl -Mopen=:std,:encoding\(UTF-8\) -ne '
    s/\e\[[0-9;?]*[ -\/]*[@-~]//g;
    if (/^test result:/) {
      if (/^test result:.*?(\d+) passed;\s+(\d+) failed;/) {
        $seen += 1;
        $failed += $2;
      }
    }
    END {
      if (!$seen) {
        print "Not available\n";
      } elsif ($failed) {
        print "Failed\n";
      } else {
        print "Passed\n";
      }
    }
  ' "${path}"
}

test_result_rows() {
  local path="$1"

  if [[ ! -f "${path}" ]]; then
    echo "| Not available | \`n/a\` | \`n/a\` | \`n/a\` |"
    return
  fi

  PERL_BADLANG=0 perl -Mopen=:std,:encoding\(UTF-8\) -ne '
    s/\e\[[0-9;?]*[ -\/]*[@-~]//g;
    if (/^\s*Running\s+(.+?)(?: \(|$)/) {
      $suite = $1;
      next;
    }
    if (/^test result:\s+(\w+)\.\s+(\d+) passed;\s+(\d+) failed;\s+(\d+) ignored/) {
      $rows += 1;
      $suite = "cargo test" if !defined($suite) || $suite eq "";
      $suite =~ s/\|/\\|/g;
      my $result = $1 eq "ok" && $3 == 0 ? "Passed" : "Failed";
      print "| `$suite` | $result | `$2 passed` | `$3 failed` | `$4 ignored` |\n";
      $suite = "";
    }
    END {
      if (!$rows) {
        print "| Not available | n/a | `n/a` | `n/a` | `n/a` |\n";
      }
    }
  ' "${path}"
}

coverage_line_percent() {
  local path="$1"

  if [[ -f "${path}" ]]; then
    awk '/^TOTAL[[:space:]]/ { line = $10 } END { print line ? line : "n/a" }' "${path}"
  else
    echo "n/a"
  fi
}

coverage_summary_row() {
  local path="$1"

  if [[ ! -f "${path}" ]]; then
    echo "| TOTAL | \`n/a\` | \`n/a\` | \`n/a\` |"
    return
  fi

  awk '
    /^TOTAL[[:space:]]/ {
      regions = $4
      functions = $7
      lines = $10
    }
    END {
      if (lines == "") {
        print "| TOTAL | `n/a` | `n/a` | `n/a` |"
      } else {
        printf "| TOTAL | `%s` | `%s` | `%s` |\n", lines, functions, regions
      }
    }
  ' "${path}"
}

coverage_gate_status() {
  local path="$1"

  if [[ ! -f "${path}" ]]; then
    echo "Not available"
    return
  fi

  awk '
    /^TOTAL[[:space:]]/ {
      line = $10
      gsub(/%/, "", line)
    }
    END {
      if (line == "") {
        print "Not available"
      } else if (line + 0 >= 95) {
        print "Passed"
      } else {
        print "Failed"
      }
    }
  ' "${path}"
}

lowest_coverage_rows() {
  local path="$1"

  if [[ ! -f "${path}" ]]; then
    echo '| Not available | `n/a` | `n/a` | `n/a` |'
    return
  fi

  PERL_BADLANG=0 perl -Mopen=:std,:encoding\(UTF-8\) -ne '
    s/\e\[[0-9;?]*[ -\/]*[@-~]//g;
    next if /^Filename\s+/ || /^-+$/ || /^TOTAL\s+/ || /^\s*$/;
    my @fields = split;
    next unless @fields >= 10 && $fields[9] =~ /%$/;
    my $line = $fields[9];
    $line =~ s/%//;
    push @rows, [$fields[0], $fields[7], $fields[8], $fields[9], $line + 0];
    END {
      @rows = sort { $a->[4] <=> $b->[4] } @rows;
      splice @rows, 5 if @rows > 5;
      if (!@rows) {
        print "| Not available | `n/a` | `n/a` | `n/a` |\n";
        exit;
      }
      for my $row (@rows) {
        print "| `$row->[0]` | `$row->[1]` | `$row->[2]` | `$row->[3]` |\n";
      }
    }
  ' "${path}"
}

has_test_failure() {
  local path="$1"

  [[ ! -f "${path}" ]] && return 0

  PERL_BADLANG=0 perl -Mopen=:std,:encoding\(UTF-8\) -ne '
    s/\e\[[0-9;?]*[ -\/]*[@-~]//g;
    if (/^test result:/) {
      if (/^test result:.*?(\d+) passed;\s+(\d+) failed;/) {
        $seen += 1;
        $failed += $2;
      }
    }
    END {
      exit((!$seen || $failed) ? 0 : 1);
    }
  ' "${path}"
}

has_coverage_failure() {
  [[ "$(coverage_gate_status "$1")" != "Passed" ]]
}

pr_number="$(resolve_pr_number)"
run_url="${GITHUB_SERVER_URL}/${GITHUB_REPOSITORY}/actions/runs/${GITHUB_RUN_ID}"
commit_sha="$(resolve_commit_sha)"
overall_test_status="$(test_status "${test_report}")"
line_coverage="$(coverage_line_percent "${coverage_report}")"
coverage_status="$(coverage_gate_status "${coverage_report}")"

comment_file="$(mktemp)"
trap 'rm -f "${comment_file}"' EXIT

{
  echo "${marker}"
  echo "## CI Test and Coverage Report"
  echo
  echo "| Field | Value |"
  echo "| --- | --- |"
  echo "| Commit | \`${commit_sha}\` |"
  echo "| Workflow run | [${GITHUB_RUN_ID}](${run_url}) |"
  echo "| Tests | ${overall_test_status} |"
  echo "| Line coverage | \`${line_coverage}\` |"
  echo "| Coverage gate | ${coverage_status} (\`>= 95%\`) |"
  echo
  echo "### Test Suites"
  echo
  echo "| Suite | Result | Passed | Failed | Ignored |"
  echo "| --- | --- | ---: | ---: | ---: |"
  test_result_rows "${test_report}"
  echo
  echo "### Coverage"
  echo
  echo "| Scope | Line Cover | Function Cover | Region Cover |"
  echo "| --- | ---: | ---: | ---: |"
  coverage_summary_row "${coverage_report}"
  echo
  echo "### Lowest Line Coverage"
  echo
  echo "| File | Lines | Missed | Line Cover |"
  echo "| --- | ---: | ---: | ---: |"
  lowest_coverage_rows "${coverage_report}"
  echo
  if has_test_failure "${test_report}" || has_coverage_failure "${coverage_report}"; then
    echo "<details>"
    echo "<summary>Failure output</summary>"
    echo
    echo '```text'
    print_clean_report_tail "${test_report}" 80
    print_clean_report_tail "${coverage_report}" 80
    echo '```'
    echo
    echo "</details>"
  else
    echo "Full raw logs are available in the workflow run."
  fi
} > "${comment_file}"

existing_comment_id="$(
  gh api "repos/${GITHUB_REPOSITORY}/issues/${pr_number}/comments" \
    --jq ".[] | select(.body | contains(\"${marker}\")) | .id" \
    | tail -n 1
)"

if [[ -n "${existing_comment_id}" ]]; then
  response="$(
    jq -n --rawfile body "${comment_file}" '{body: $body}' \
      | gh api --method PATCH "repos/${GITHUB_REPOSITORY}/issues/comments/${existing_comment_id}" --input -
  )"
else
  response="$(
    jq -n --rawfile body "${comment_file}" '{body: $body}' \
      | gh api --method POST "repos/${GITHUB_REPOSITORY}/issues/${pr_number}/comments" --input -
  )"
fi

comment_id="$(jq -r '.id // empty' <<< "${response}")"

if [[ -z "${comment_id}" ]]; then
  echo "::error::GitHub did not return a comment id for the CI report."
  exit 1
fi

if ! gh api "repos/${GITHUB_REPOSITORY}/issues/comments/${comment_id}" --jq '.body' | grep -Fq "${marker}"; then
  echo "::error::CI report comment ${comment_id} was not persisted on the pull request."
  exit 1
fi

echo "Updated CI report comment ${comment_id} on PR ${pr_number}."
