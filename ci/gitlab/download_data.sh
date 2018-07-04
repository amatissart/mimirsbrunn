#! /bin/bash
set -e
set -x

# Download addresses
wget http://bano.openstreetmap.fr/data/full.csv.gz -P $ADDR_DIR
gunzip $ADDR_DIR/full.csv.gz

# Download osm dataset
wget https://download.geofabrik.de/europe/france-latest.osm.pbf -P $OSM_DIR
