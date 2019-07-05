use docker_wrapper::*;
use serde_json::value::Value;
use serde_json::Map;
#[macro_use]
extern crate slog;
#[macro_use]
extern crate slog_scope;

pub struct ElasticSearchWrapper<'a> {
    pub docker_wrapper: &'a DockerWrapper,
    pub rubber: mimir::rubber::Rubber,
}

impl<'a> ElasticSearchWrapper<'a> {
    pub fn host(&self) -> String {
        self.docker_wrapper.host()
    }

    pub fn init(&mut self) {
        self.rubber.delete_index(&"_all".to_string()).unwrap();
    }

    //    A way to watch if indexes are built might be curl http://localhost:9200/_stats
    //    then _all/total/segments/index_writer_memory_in_bytes( or version_map_memory_in_bytes)
    //    should be == 0 if indexes are ok (no refresh needed)
    pub fn refresh(&self) {
        info!("Refreshing ES indexes");

        let res = reqwest::Client::new()
            .get(&format!("{}/_refresh", self.host()))
            .send()
            .unwrap();
        assert!(
            res.status() == reqwest::StatusCode::OK,
            "Error ES refresh: {:?}",
            res
        );
    }

    pub fn new(docker_wrapper: &DockerWrapper) -> ElasticSearchWrapper<'_> {
        let mut es_wrapper = ElasticSearchWrapper {
            docker_wrapper: docker_wrapper,
            rubber: mimir::rubber::Rubber::new(&docker_wrapper.host()),
        };
        es_wrapper.init();
        es_wrapper
    }

    /// count the number of documents in the index
    /// If you want to count eg the number of POI, you would call
    /// es_wrapper.count("_type:POI")
    pub fn count(&self, word: &str) -> u64 {
        info!("counting documents with munin/_count?q={}", word);
        let mut res = self
            .rubber
            .get(&format!("munin/_count?q={}", word))
            .unwrap();
        assert!(res.status() == reqwest::StatusCode::OK);
        let json: serde_json::Value = res.json().unwrap();
        json["count"].as_u64().unwrap()
    }

    /// simple search on an index
    /// assert that the result is OK and transform it to a json Value
    pub fn search(&self, word: &str) -> serde_json::Value {
        let mut res = self
            .rubber
            .get(&format!("munin/_search?q={}", word))
            .unwrap();
        assert!(res.status() == reqwest::StatusCode::OK);
        res.json().unwrap()
    }

    pub fn search_on_global_stop_index(&self, word: &str) -> serde_json::Value {
        let mut res = self
            .rubber
            .get(&format!("munin_global_stops/_search?q={}", word))
            .unwrap();
        assert!(res.status() == reqwest::StatusCode::OK);
        res.json().unwrap()
    }

    pub fn search_and_filter<'b, F>(
        &self,
        word: &str,
        predicate: F,
    ) -> impl Iterator<Item = mimir::Place> + 'b
    where
        F: 'b + FnMut(&mimir::Place) -> bool,
    {
        self.search_and_filter_on_index(word, predicate, false)
    }

    pub fn search_and_filter_on_global_stop_index<'b, F>(
        &self,
        word: &str,
        predicate: F,
    ) -> impl Iterator<Item = mimir::Place> + 'b
    where
        F: 'b + FnMut(&mimir::Place) -> bool,
    {
        self.search_and_filter_on_index(word, predicate, true)
    }

    fn search_and_filter_on_index<'b, F>(
        &self,
        word: &str,
        predicate: F,
        search_on_global_stops: bool,
    ) -> impl Iterator<Item = mimir::Place> + 'b
    where
        F: 'b + FnMut(&mimir::Place) -> bool,
    {
        use serde_json::map::Entry;

        fn into_object(json: Value) -> Option<Map<String, Value>> {
            match json {
                Value::Object(o) => Some(o),
                _ => None,
            }
        }
        fn get(json: Value, key: &str) -> Option<Value> {
            into_object(json).and_then(|mut json| match json.entry(key.to_string()) {
                Entry::Occupied(o) => Some(o.remove()),
                _ => None,
            })
        }
        let json = if search_on_global_stops {
            self.search_on_global_stop_index(word)
        } else {
            self.search(word)
        };
        get(json, "hits")
            .and_then(|json| get(json, "hits"))
            .and_then(|hits| match hits {
                Value::Array(v) => Some(v),
                _ => None,
            })
            .unwrap_or_else(Vec::default)
            .into_iter()
            .filter_map(|json| {
                into_object(json).and_then(|obj| {
                    let doc_type = obj
                        .get("_type")
                        .and_then(|doc_type| doc_type.as_str())
                        .map(|doc_type| doc_type.into());

                    doc_type.and_then(|doc_type| {
                        // The real object is contained in the _source section.
                        obj.get("_source").and_then(|src| {
                            bragi::query::make_place(doc_type, Some(Box::new(src.clone())))
                        })
                    })
                })
            })
            .filter(predicate)
    }
}

pub struct BragiHandler {
    app: actix_http_test::TestServerRuntime,
}

impl BragiHandler {
    pub fn new(url: String) -> BragiHandler {
        let ctx = bragi::Context::from(&bragi::Args {
            connection_string: url.clone(),
            ..Default::default()
        });

        let prometheus = actix_web_prom::PrometheusMetrics::new("api", "/metrics"); //TODO don't forget to add in_flight queries
        let srv = actix_http_test::TestServer::new(move || {
            actix_http::HttpService::new(
                actix_web::App::new()
                    .data(ctx.clone())
                    .wrap(actix_cors::Cors::new().allowed_methods(vec!["GET"]))
                    .wrap(prometheus.clone())
                    // .wrap(prometheus_middleware::PrometheusMiddleware::default())
                    .wrap(actix_web::middleware::Logger::default())
                    .configure(bragi::server::configure_server)
                    .default_service(
                        actix_web::web::resource("")
                            .route(actix_web::web::get().to(bragi::server::default_404)),
                    ),
            )
        });

        BragiHandler { app: srv }
    }

    pub fn raw_get(&mut self, q: &str) -> (actix_http::http::StatusCode, bytes::Bytes) {
        let q = url_encode(q);
        let req = self.app.get(q);

        let mut r = self.app.block_on(req.send()).unwrap();

        let status = r.status();

        // TODO: at one point it would be nice to read the body only if we need it,
        // but for the moment I'm not able to return a future here
        let body = self.app.block_on(r.body()).unwrap();
        (status, body)
    }

    pub fn get_status(&mut self, q: &str) -> actix_http::http::StatusCode {
        let q = url_encode(q);
        let req = self.app.get(q);

        let r = self.app.block_on(req.send()).unwrap();
        r.status()
    }

    pub fn get(&mut self, q: &str) -> Vec<Map<String, Value>> {
        let j = self.get_json(q);
        self.get_results(j)
    }

    pub fn get_json(&mut self, q: &str) -> Value {
        let (status, s) = self.raw_get(q);
        assert!(status.is_success(), "invalid status: {}", status);

        self.to_json(s)
    }

    pub fn get_unchecked_json(&mut self, q: &str) -> (actix_web::http::StatusCode, Value) {
        let (status, s) = self.raw_get(q);

        (status, self.to_json(s))
    }

    pub fn raw_post(
        &mut self,
        q: &str,
        shape: &'static str,
    ) -> (actix_http::http::StatusCode, bytes::Bytes) {
        let q = url_encode(q);
        let mut r = self
            .app
            .block_on(
                self.app
                    .post(q)
                    .header(actix_web::http::header::CONTENT_TYPE, "application/json")
                    .send_body(shape),
            )
            .unwrap_or_else(|e| panic!("impossible to query bragi: {}", e));

        let status = r.status();

        // TODO: at one point it would be nice to read the body only if we need it,
        // but for the moment I'm not able to return a future here
        let body = self.app.block_on(r.body()).unwrap();
        (status, body)
    }

    pub fn post(&mut self, q: &str, shape: &'static str) -> Vec<Map<String, Value>> {
        let j = self.post_as_json(q, shape);
        self.get_results(j)
    }

    pub fn post_as_json(&mut self, q: &str, shape: &'static str) -> Value {
        let (status, s) = self.raw_post(q, shape);
        assert!(status.is_success(), "invalid status: {}", status);

        self.to_json(s)
    }

    pub fn to_json(&mut self, b: bytes::Bytes) -> Value {
        let body = std::str::from_utf8(&b).unwrap();
        serde_json::from_str(body).unwrap()
    }

    fn get_results(&mut self, response: Value) -> Vec<Map<String, Value>> {
        response
            .pointer("/features")
            .expect("wrongly formated bragi response")
            .as_array()
            .expect("features must be array")
            .iter()
            .map(|f| {
                f.pointer("/properties/geocoding")
                    .expect("no field in bragi response")
                    .as_object()
                    .unwrap()
                    .clone()
            })
            .collect()
    }
}

fn url_encode(q: &str) -> String {
    let q: String =
        url::percent_encoding::utf8_percent_encode(q, url::percent_encoding::DEFAULT_ENCODE_SET)
            .collect();
    q.replace("%3F", "?")
}
