#define NOMINMAX
#include <windows.h>
#include <iostream>
#include <string>
#include <vector>
#include <curl/curl.h>
#include <nlohmann/json.hpp>
#include <thread>
#include <algorithm>
#include <chrono>

static size_t WriteCallback(void* contents, size_t size, size_t nmemb, void* userp) {
    size_t realsize = size * nmemb;
    auto* buffer = static_cast<std::string*>(userp);
    buffer->append(static_cast<char*>(contents), realsize);
    return realsize;
}

std::vector<std::string> getPerpetualFuturesSymbols() {
    CURL* curl = curl_easy_init();
    std::vector<std::string> perpetualSymbols;

    if (curl) {
        std::string url = "https://fapi.binance.com/fapi/v1/exchangeInfo";
        std::string readBuffer;

        curl_easy_setopt(curl, CURLOPT_URL, url.c_str());
        curl_easy_setopt(curl, CURLOPT_WRITEFUNCTION, WriteCallback);
        curl_easy_setopt(curl, CURLOPT_WRITEDATA, &readBuffer);
        CURLcode res = curl_easy_perform(curl);

        if (res == CURLE_OK) {
            auto json_data = nlohmann::json::parse(readBuffer);
            for (const auto& symbol : json_data["symbols"]) {
                if (symbol["contractType"] == "PERPETUAL" && symbol["quoteAsset"] == "USDT" && symbol["marginAsset"] == "USDT") {

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



                    if (symbol["status"] == "TRADING") { // When symbol["status"] is "trading".

                        if () { // When the 'Symbol_Info' table exists.
                            if () { // If the difference between the current time and the first column (Open Time) of the last row in the 'Symbol_Klines_dataset' table is less than 98 minutes.
                                // Store the remaining data collected from the endpoint into the 'Symbol_Info' table (e.g., minQty, stepSize, underlyingSubType, etc.).
                                if () { // When the status of 'Symbol_Info' is not "trading".
                                    // Update the status of 'Symbol_Info' to "trading".
                                }
                            }
                            else () { // If the difference between the current time and the Open Time value of the second-to-last row in the 'Symbol_Klines_dataset' table is 98 minutes or more.
                                // Change the status of 'Symbol_Info' to "accumulating".
                                // Insert the Open Time value of the second-to-last row in the 'Symbol_Klines_dataset' table into the onboardDate of 'Symbol_Info'.
                            }
                        }
                        else { // When the table does not exist.
                            int onboardData_for_table = symbol["onboardDate"]; // Retrieve the symbol's listing time.
                            if () { // If the difference between the current time and the onboardData_for_table time is less than 98 minutes.
                                // Create the 'Symbol_Info' table.
                                // Change the status of 'Symbol_Info' to "trading".
                                // Store the remaining data collected from the endpoint into the 'Symbol_Info' table (e.g., minQty, stepSize, underlyingSubType, etc.).
                            }
                            else () { // If the difference between the current time and the onboardData_for_table time is 98 minutes or more.
                                // Create the 'Symbol_Info' table.
                                // Change the status of 'Symbol_Info' to "accumulating".
                                // Insert the onboardData_for_table value into the onboardDate of 'Symbol_Info'.
                            }
                        }
                        perpetualSymbols.push_back(symbol["symbol"]);
                    }
                    else { // // When symbol["status"] is not "trading"
                        if () { // When the 'Symbol_Info' table exists.
                            if () { // When the status of 'Symbol_Info' is not Null.
                                // Change the status of 'Symbol_Info' to Null.
                            }
                        }
                    }
                }
            }
        }
        else {
            std::cerr << "cURL request failed: " << curl_easy_strerror(res) << std::endl;
        }
        curl_easy_cleanup(curl);
    }
    return perpetualSymbols;
}

std::string fetchKlines(const std::string& symbol, const std::string& interval, int limit) {
    CURL* curl = curl_easy_init();
    std::string readBuffer;

    if () { // When the status of 'Symbol_Info' is "trading".
        if (curl) {
            std::string url = "https://fapi.binance.com/fapi/v1/klines?symbol=" + symbol +
                "&interval=" + interval + "&limit=" + std::to_string(limit);
        }
    }
    else { // When the status of 'Symbol_Info' is "accumulating".
        if (curl) {
            startTime = // Initialize with the onboardDate value of 'Symbol_Info'.
                std::string url = "https://fapi.binance.com/fapi/v1/klines?symbol=" + symbol +
                "&interval=" + interval + "&limit=" + std::to_string(limit) +
                "&startTime=" + std::to_string(startTime);
        }
    }

    curl_easy_setopt(curl, CURLOPT_URL, url.c_str());
    curl_easy_setopt(curl, CURLOPT_WRITEFUNCTION, WriteCallback);
    curl_easy_setopt(curl, CURLOPT_WRITEDATA, &kindl_data);

    CURLcode res = curl_easy_perform(curl);

    if (res != CURLE_OK) {
        std::cerr << "curl_easy_perform() failed: " << curl_easy_strerror(res) << std::endl;
    }

    curl_easy_cleanup(curl);
    auto json_data = nlohmann::json::parse(kindl_data);

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



    return kindl_data;
}

inline void processSymbols(const std::vector<std::string>& symbols) {
    const std::string interval = "1m";
    const int limit = 99;

    for (const auto& symbol : symbols) {
        std::string jsonResponse = fetchKlines(symbol, interval, limit);
        if (jsonResponse.empty()) {
            std::cerr << "Failed to fetch data for symbol: " << symbol << std::endl;
        }
    }
}

void multi_threading(const std::vector<std::string>& symbols) {
    int num_threads = std::thread::hardware_concurrency();
    if (num_threads == 0) num_threads = 1;

    size_t chunk_size = (symbols.size() + num_threads - 1) / num_threads;
    std::vector<std::thread> threads;

    for (int i = 0; i < num_threads; ++i) {
        size_t start = i * chunk_size;
        size_t end = std::min(start + chunk_size, symbols.size());


        threads.emplace_back(processSymbols, std::vector<std::string>(symbols.begin() + start, symbols.begin() + end));
    }

    for (auto& thread : threads) {
        thread.join();
    }
}

int main() {
    auto start = std::chrono::high_resolution_clock::now();
    std::vector<std::string> symbols = getPerpetualFuturesSymbols();
    std::cout << symbols.size() << std::endl;
    multi_threading(symbols);
    auto end = std::chrono::high_resolution_clock::now();
    std::chrono::duration<double> elapsed = end - start;

    std::cout << "Elapsed time: " << elapsed.count() << " seconds" << std::endl;
    return 0;
}
