#define NOMINMAX
#include <windows.h>
#include <iostream>
#include <string>
#include <vector>
#include <thread>
#include <algorithm>
#include <chrono>
#include <mutex>
#include <curl/curl.h>
#define SIMDJSON_IMPLEMENTATION
#include "simdjson.h"


static std::once_flag curl_init_flag;
static size_t WriteCallback(void* contents, size_t size, size_t nmemb, void* userp) {
    size_t realsize = size * nmemb;
    auto* buffer = static_cast<std::string*>(userp);
    buffer->append(static_cast<char*>(contents), realsize);
    return realsize;
}

std::vector<std::string> getPerpetualFuturesSymbols() {
    std::call_once(curl_init_flag, []() {
        curl_global_init(CURL_GLOBAL_ALL);
        });
    std::vector<std::string> resultSymbols;
    CURL* curl = curl_easy_init();
    if (!curl) {
        return resultSymbols;
    }
    std::string url = "https://fapi.binance.com/fapi/v1/exchangeInfo";
    std::string readBuffer;
    curl_easy_setopt(curl, CURLOPT_URL, url.c_str());
    curl_easy_setopt(curl, CURLOPT_WRITEFUNCTION, WriteCallback);
    curl_easy_setopt(curl, CURLOPT_WRITEDATA, &readBuffer);
    curl_easy_setopt(curl, CURLOPT_NOSIGNAL, 1L);
    curl_easy_setopt(curl, CURLOPT_SSL_VERIFYPEER, 0L);
    curl_easy_setopt(curl, CURLOPT_SSL_VERIFYHOST, 0L);
    curl_easy_setopt(curl, CURLOPT_TCP_NODELAY, 1L);
    curl_easy_setopt(curl, CURLOPT_TCP_FASTOPEN, 1L);
    curl_easy_setopt(curl, CURLOPT_FRESH_CONNECT, 0L);
    curl_easy_setopt(curl, CURLOPT_FORBID_REUSE, 0L);

    CURLcode res = curl_easy_perform(curl);
    if (res == CURLE_OK) {
        simdjson::ondemand::parser parser;
        simdjson::ondemand::document doc = parser.iterate(readBuffer);

        simdjson::ondemand::array symbolsArr = doc["symbols"];
        for (auto sym : symbolsArr) {
            std::string_view contractType = sym["contractType"].get_string();
            std::string_view quoteAsset = sym["quoteAsset"].get_string();
            std::string_view marginAsset = sym["marginAsset"].get_string();
            std::string_view status = sym["status"].get_string();

            if (contractType == "PERPETUAL" && quoteAsset == "USDT" && marginAsset == "USDT" && status == "TRADING")
            {
                // 여기서 종목 상태를 살펴보는 코드를 집어넣어야함
                std::string_view symbol = sym["symbol"].get_string();
                resultSymbols.emplace_back(symbol);
            }
        }
    }
    else {
        std::cerr << "[getPerpetualFuturesSymbols] curl error: " << curl_easy_strerror(res) << std::endl;
    }
    curl_easy_cleanup(curl);
    return resultSymbols;
}

struct KlineRequest {
    std::string symbol;
    std::string interval;
    int limit;
    std::string response;
};

void fetchKlinesMulti(std::vector<KlineRequest>& requests) {
    CURLM* multi = curl_multi_init();
    std::vector<CURL*> easy_handles;
    easy_handles.reserve(requests.size());

    curl_multi_setopt(multi, CURLMOPT_PIPELINING, (long)CURLPIPE_HTTP1);
    curl_multi_setopt(multi, CURLMOPT_MAX_HOST_CONNECTIONS, (long)8);

    for (auto& req : requests) {
        CURL* easy = curl_easy_init();
        if (!easy) continue;

        std::string url = "https://fapi.binance.com/fapi/v1/klines?symbol=" + req.symbol + "&interval=" + req.interval + "&limit=" + std::to_string(req.limit);

        curl_easy_setopt(easy, CURLOPT_URL, url.c_str());
        curl_easy_setopt(easy, CURLOPT_WRITEFUNCTION, WriteCallback);
        curl_easy_setopt(easy, CURLOPT_WRITEDATA, &req.response);
        curl_easy_setopt(easy, CURLOPT_NOSIGNAL, 1L);
        curl_easy_setopt(easy, CURLOPT_SSL_VERIFYPEER, 0L);
        curl_easy_setopt(easy, CURLOPT_SSL_VERIFYHOST, 0L);
        curl_easy_setopt(easy, CURLOPT_TCP_NODELAY, 1L);
        curl_easy_setopt(easy, CURLOPT_TCP_FASTOPEN, 1L);
        curl_easy_setopt(easy, CURLOPT_FRESH_CONNECT, 0L);
        curl_easy_setopt(easy, CURLOPT_FORBID_REUSE, 0L);
        curl_multi_add_handle(multi, easy);
        easy_handles.push_back(easy);
    }

    int still_running = 0;
    curl_multi_perform(multi, &still_running);

    while (still_running) {
        curl_multi_poll(multi, nullptr, 0, 100, nullptr);
        curl_multi_perform(multi, &still_running);
    }

    // 파싱 코드를 넣자

    for (auto eh : easy_handles) {
        curl_multi_remove_handle(multi, eh);
        curl_easy_cleanup(eh);
    }
    curl_multi_cleanup(multi);
}

void multi_threading(const std::vector<std::string>& symbols, const std::string& interval = "1m", int limit = 99) { // limit는 절대 건들면 안됨. 99개 이하로는 조정 가능함
    int num_threads = std::thread::hardware_concurrency();
    if (num_threads == 0) num_threads = 1;
    num_threads = std::min<int>(num_threads, symbols.size());
    if (num_threads <= 0) return;

    std::vector<std::thread> threads;
    threads.reserve(num_threads);

    size_t base = symbols.size() / num_threads;
    size_t rem = symbols.size() % num_threads;
    size_t start = 0;

    for (int i = 0; i < num_threads; ++i) {
        size_t chunk = base + (i < rem ? 1 : 0);
        size_t end = start + chunk;

        std::vector<std::string> subset(symbols.begin() + start, symbols.begin() + end);
        start = end;

        std::vector<KlineRequest> requests;
        requests.reserve(subset.size());
        for (auto& sym : subset) {
            KlineRequest rq;
            rq.symbol = sym;
            rq.interval = interval;
            rq.limit = limit;
            requests.push_back(std::move(rq));
        }

        threads.emplace_back([requests = std::move(requests)]() mutable {
            fetchKlinesMulti(requests);
            });
    }

    for (auto& th : threads) {
        th.join();
    }
}

int main() {
    auto t0 = std::chrono::high_resolution_clock::now();

    std::vector<std::string> symbols = getPerpetualFuturesSymbols();
    std::cout << "Found " << symbols.size() << " symbols.\n";

    multi_threading(symbols, "1m", 99);

    auto t1 = std::chrono::high_resolution_clock::now();
    double elapsed = std::chrono::duration<double>(t1 - t0).count();

    std::cout << "Elapsed time: " << elapsed << " sec\n";
    curl_global_cleanup();
    return 0;
}
