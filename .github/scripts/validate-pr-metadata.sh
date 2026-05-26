#!/usr/bin/env bash
set -euo pipefail

title="${PR_TITLE:?PR_TITLE is required}"
body="${PR_BODY:-}"
commit_count="${PR_COMMIT_COUNT:?PR_COMMIT_COUNT is required}"

fail() {
  echo "::error::$1"
  exit 1
}

require_english_text() {
  local label="$1"
  local value="$2"

  if PERL_BADLANG=0 perl -Mopen=:std,:encoding\(UTF-8\) -e 'local $/; my $text = <STDIN>; exit($text =~ /[^\W\d_A-Za-z_]/ ? 0 : 1);' <<< "${value}"; then
    fail "${label} must be written in English; emoji are allowed, but non-English letters are not."
  fi
}

if [[ "${commit_count}" != "1" ]]; then
  fail "Each pull request must contain exactly one commit; found ${commit_count}."
fi

require_english_text "Pull request title" "${title}"
require_english_text "Pull request body" "${body}"

if [[ "${#title}" -lt 8 ]]; then
  fail "Pull request title is too short to describe the change."
fi

shopt -s nocasematch
if [[ "${title}" =~ ^(update|changes|misc|wip|fix|feature|refactor)$ ]]; then
  fail "Pull request title must describe the actual change, not a generic label."
fi
shopt -u nocasematch

required_sections=(
  "## 1. Requirement"
  "## 2. Implementation"
  "## 3. Architecture and Functional Impact"
  "## 4. Test and Verification Method"
  "## 5. Test Results"
  "### Functional Tests"
  "### Benchmark Tests"
  "### End-to-End Tests"
)

for section in "${required_sections[@]}"; do
  if [[ "${body}" != *"${section}"* ]]; then
    fail "Pull request body is missing required section: ${section}"
  fi
done

template_placeholders=(
  "State the concrete user or product requirement."
  "Describe how the change is implemented."
  "Describe the impact on existing architecture and behavior."
  "List the commands, scenarios, and review methods used."
  "Include command, environment, baseline, new result, and interpretation."
  "Include scenario, command or workflow, and result."
)

for placeholder in "${template_placeholders[@]}"; do
  if [[ "${body}" == *"${placeholder}"* ]]; then
    fail "Pull request body still contains template placeholder: ${placeholder}"
  fi
done
