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

            if (contractType == "PERPETUAL" && quoteAsset == "USDT" && marginAsset == "USDT") {

                // process 1

                // path :: "Market_Data_Base\<Symbol_Name>\Symbol_Klines_dataset" this is the 'Symbol_Klines_dataset' table's path
                // path :: "Market_Data_Base\<Symbol_Name>\Symbol_Info" this is the 'Symbol_Info' table's path

                // "Status, minQty, stepSize, underlyingSubType, pricePrecision, quantityPrecision, Notional, onboardDate" Row: 1 �� Column: 11 this is the 'Symbol_Info' table structure

                // In Process 1, the most important values are the status and onboardDate. The status is divided into three categories: (trading, accumulating, Null). 
                // If the status is "trading," it indicates that the asset is recognized as tradable and trading will proceed. 
                // If the status is "accumulating," it means that there is no market data available for the current time, so trading is paused until all necessary data is collected. 
                // "Null" indicates that the asset has been delisted or trading has been halted.
                // The onboardDate is the listing time data provided by Binance for each asset. When the status is "accumulating," if the asset's data has never been collected before, trading data collection will begin from the onboardDate time.

                // The onboardDate is provided in milliseconds. If it needs to be compared with the current time, it must be converted accordingly. 

                // If you're curious about the structure or contents of the data retrieved from the endpoint in Process 1, run the Python script named "a" included with this project. It will fetch and display sample data for Bitcoin.


                if (status == "TRADING")
                {
                    //if () { // When the 'Symbol_Info' table exists.
                    //    if () { // If the difference between the current time and the first column (Open Time) of the last row in the 'Symbol_Klines_dataset' table is less than 98 minutes.
                    //        // Store the remaining data collected from the endpoint into the 'Symbol_Info' table (e.g., minQty, stepSize, underlyingSubType, etc.).
                    //        if () { // When the status of 'Symbol_Info' is not "trading".
                    //            // Update the status of 'Symbol_Info' to "trading".
                    //        }
                    //    }
                    //    else () { // If the difference between the current time and the Open Time value of the second-to-last row in the 'Symbol_Klines_dataset' table is 98 minutes or more.
                    //        // Change the status of 'Symbol_Info' to "accumulating".
                    //        // Insert the Open Time value of the second-to-last row in the 'Symbol_Klines_dataset' table into the onboardDate of 'Symbol_Info'.
                    //    }
                    //}
                    //else { // When the table does not exist.
                    //    int onboardData_for_table = symbol["onboardDate"]; // Retrieve the symbol's listing time.
                    //    if () { // If the difference between the current time and the onboardData_for_table time is less than 98 minutes.
                    //        // Create the 'Symbol_Info' table.
                    //        // Change the status of 'Symbol_Info' to "trading".
                    //        // Store the remaining data collected from the endpoint into the 'Symbol_Info' table (e.g., minQty, stepSize, underlyingSubType, etc.).
                    //    }
                    //    else () { // If the difference between the current time and the onboardData_for_table time is 98 minutes or more.
                    //        // Create the 'Symbol_Info' table.
                    //        // Change the status of 'Symbol_Info' to "accumulating".
                    //        // Insert the onboardData_for_table value into the onboardDate of 'Symbol_Info'.
                    //    }
                    //}

                    std::string_view symbol = sym["symbol"].get_string();
                    resultSymbols.emplace_back(symbol);

                }

                //else { // // When symbol["status"] is not "trading"
                //    if () { // When the 'Symbol_Info' table exists.
                //        if () { // When the status of 'Symbol_Info' is not Null.
                //            // Change the status of 'Symbol_Info' to Null.
                //        }
                //    }
                //}
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


        // if () { // When the status of 'Symbol_Info' is "trading".
            std::string url = "https://fapi.binance.com/fapi/v1/klines?symbol=" + req.symbol + "&interval=" + req.interval + "&limit=" + std::to_string(req.limit);
        // }
        // else { // When the status of 'Symbol_Info' is "accumulating".
            // startTime = // Initialize with the onboardDate value of 'Symbol_Info'.
            // std::string url = "https://fapi.binance.com/fapi/v1/klines?symbol=" + req.symbol + "&interval=" + req.interval + "&limit=" + "&startTime=" + std::to_string(startTime);
        // }


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

    // process 2

    // This is the section for writing code related to (Table) 1. "Symbol Klines dataset" from blueprint slide 5, with Row: N/A �� Column: 11.
    // ------------------------------------------------------------------
    // Write code that sets kline[0] as the indexing column. If the table does not exist, create a new one and add the data. 
    // If the table already exists, remove the last row of the table and then append the new dataset. Ensure that rows with duplicate kline[0] values are ignored.
    // 
    // For example, if the Symbol Klines dataset table exists and contains 1,293 rows, remove the last row to reduce it to 1,292 rows. 
    // Then, append the new dataset based on the indexing column Open Time, ensuring no duplicate entries are added.
    // ------------------------------------------------------------------
    // kline[0] 'Open Time' , kline[1] 'Open' , kline[2] 'High' , kline[3] 'Low' , kline[4] 'Close' , kline[5] 'Volume' , kline[6] 'Close Time' , kline[7] 'Quote Asset Volume' , kline[8] 'Number of Trades' , kline[9] 'Taker Buy Base Asset Volume' , kline[10] 'Taker Buy Quote Asset Volume'


    for (auto eh : easy_handles) {
        curl_multi_remove_handle(multi, eh);
        curl_easy_cleanup(eh);
    }
    curl_multi_cleanup(multi);
}

void multi_threading(const std::vector<std::string>& symbols,const std::string& interval = "1m",int limit = 99){ // limit는 절대 건들면 안됨. 99개 이하로는 조정 가능함
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
