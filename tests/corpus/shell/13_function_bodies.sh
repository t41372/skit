#!/usr/bin/env bash
GLOBAL_LABEL=production
deploy() {
  local target=staging
  ENVIRONMENT=dev
  echo "$target $ENVIRONMENT $GLOBAL_LABEL"
}
deploy
