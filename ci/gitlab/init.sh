#! /bin/bash
set -e
set -x

apk add --no-cache python3 py3-pip git bash
pip3 install docker-compose pipenv
git clone -b esTimeout https://github.com/QwantResearch/docker_mimir.git
mv ci/gitlab/*.yml docker_mimir/
cd docker_mimir && pipenv install --system --deploy
