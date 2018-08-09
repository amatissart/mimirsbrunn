#! /bin/bash
set -e
set -x

# Download addresses
mkdir -p $ADDR_DIR
BANO_FILE=$ADDR_DIR/full.csv
if [ ! -f $BANO_FILE ]; then
    wget http://bano.openstreetmap.fr/data/full.csv.gz -P $ADDR_DIR
    gunzip $ADDR_DIR/full.csv.gz
else
    echo "No addr download: $BANO_FILE already exists"
fi

# Download osm dataset
mkdir -p $OSM_DIR
OSM_FILE=$OSM_DIR/france-latest.osm.pbf
if [ ! -f $OSM_FILE ]; then
    wget https://download.geofabrik.de/europe/france-latest.osm.pbf -O $OSM_FILE
else
    echo "No osm download: $OSM_FILE already exists"
fi
