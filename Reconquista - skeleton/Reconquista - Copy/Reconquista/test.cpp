/*
  Compile with (example):
    cl /std:c++17 /EHsc /O2 /DNDEBUG ws_example.cpp /link Ws2_32.lib User32.lib Winmm.lib
*/

#pragma comment(lib, "winmm.lib")

#define ASIO_STANDALONE
#include <websocketpp/config/asio_client.hpp>
#include <websocketpp/client.hpp>
#include <asio/ssl.hpp>
#include <numeric>
#include <simdjson.h>
#include <windows.h>
#include <mmsystem.h>
#include <iostream>
#include <string>
#include <thread>
#include <asio.hpp>
#include <curl/curl.h>
#include <unordered_map>
#include <stdexcept>
#include <cmath>
#include <algorithm>
#include <memory>
#include <queue>
#include <mutex>
#include <cstring>
#include <condition_variable>
#include <atomic>
#include <cstdlib>
#include <charconv>
#include <sstream>
#include <string_view>
#include <vector>
#include <deque>
#include <chrono>
#include <asio/ssl/context.hpp>
#include "concurrentqueue.h"
//#include <torch/script.h> 


namespace ssl = asio::ssl;

// 전역변수들 ===============================================================
static std::atomic<bool>          g_loggerRunning{ false };
static std::thread                g_loggerThread;
static moodycamel::ConcurrentQueue<std::string> g_logQueue;
static bool g_havePosition = false;
//std::string model_path = "D:\\workspace\\Reconquista\\models\\model.pt";
//torch::jit::script::Module model;
// =========================================================================

// 로거를 시작(쓰레드 생성)
void logInit() {
    g_loggerRunning.store(true, std::memory_order_release);
    g_loggerThread = std::thread([] {
        using namespace std::chrono_literals;
        const auto SLEEP_DURATION = 1ms;
        std::vector<std::string> batch;
        batch.reserve(256);
        while (true) {
            batch.clear();
            std::string msg;
            while (g_logQueue.try_dequeue(msg)) {
                batch.push_back(std::move(msg));
            }
            if (batch.empty()) {
                if (!g_loggerRunning.load(std::memory_order_acquire)) {
                    break;
                }
                std::this_thread::sleep_for(SLEEP_DURATION);
                continue;
            }
            for (auto& m : batch) {
                std::cout << m;
            }
            std::cout << std::flush;
            std::this_thread::sleep_for(SLEEP_DURATION);
        }
        });
}
void logStop() {
    g_loggerRunning.store(false, std::memory_order_release);
    if (g_loggerThread.joinable()) {
        g_loggerThread.join();
    }
}
void logMessage(const std::string& msg) {
    g_logQueue.enqueue(msg);
}
struct TradeData {
    double  price = 0.0;
    double  quantity = 0.0;
    int64_t tradetime = 0;
    bool side{};
};

// 링버퍼 ===============================================================


class RingBuffer {
public:
    explicit RingBuffer(size_t capacity)
        : m_capacity(capacity),
        m_data(capacity)
    {
        // startIdx와 endIdx는 0부터 시작
    }

    void push_back(const TradeData& td) {
        m_data[m_endIdx % m_capacity] = td;
        m_endIdx++;

        if (m_endIdx - m_startIdx > m_capacity) {
            m_startIdx = m_endIdx - m_capacity;
        }
    }

    inline void discardOld(int64_t nowMs, int64_t time) {
        while (m_startIdx < m_endIdx) {
            size_t idx = m_startIdx % m_capacity;
            int64_t tms = m_data[idx].tradetime;
            if (nowMs - tms > time) {
                m_startIdx++;
            }
            else {
                break;
            }
        }
    }

    // 3초 이내인 요소 개수
    size_t size() const {
        return m_endIdx - m_startIdx;
    }

    // 나중에 iterator를 노출하도록 만들어야 할듯

    std::vector<TradeData> getSnapshot() const {
        std::vector<TradeData> out;
        out.reserve(size_t(size()));

        for (size_t i = m_startIdx; i < m_endIdx; i++) {
            out.push_back(m_data[i % m_capacity]);
        }
        return out;
    }

private:
    const size_t         m_capacity;
    std::vector<TradeData> m_data;
    // ring buffer 인덱스
    size_t               m_startIdx = 0;
    size_t               m_endIdx = 0;
};

// 메인 클래스 ===============================================================

class reconquistaConclusion {
public:
    using TradeQueue = moodycamel::ConcurrentQueue<TradeData>;
    using client = websocketpp::client<websocketpp::config::asio_tls_client>;

    explicit reconquistaConclusion(const std::string& symbol)
        : m_uri("wss://fstream.binance.com/ws/" + symbol + "@trade"),

        temporary_ringBuffer(300'000),
        shortsave_buy_ringBuffer(300'000),
        //longsave_sell_ringBuffer(1'000'000),
        longsave_all_ringBuffer(1'000'000),
        midsave_buy_ringBuffer(300'000),
        midsave_sell_ringBuffer(300'000)
    {
        conclusion_client.clear_access_channels(websocketpp::log::alevel::all);
        conclusion_client.clear_error_channels(websocketpp::log::elevel::all);
    }

    void start() {
        try {
            conclusion_client.init_asio();
            conclusion_client.set_tls_init_handler(
                [](websocketpp::connection_hdl)
                -> std::shared_ptr<asio::ssl::context> {
                    auto ctx = std::make_shared<asio::ssl::context>(asio::ssl::context::tlsv12);
                    ctx->set_options(
                        asio::ssl::context::default_workarounds |
                        asio::ssl::context::no_sslv2 |
                        asio::ssl::context::no_sslv3 |
                        asio::ssl::context::single_dh_use
                    );
                    return ctx;
                }
            );
            conclusion_client.set_message_handler(
                [this](websocketpp::connection_hdl, client::message_ptr msg) {
                    onMessage(msg);
                }
            );

            websocketpp::lib::error_code ec;
            auto con = conclusion_client.get_connection(m_uri, ec);
            if (ec) {
                logMessage("[Error] get_connection(trade): " + ec.message() + "\n");
                return;
            }
            conclusion_client.connect(con);

            conclusion_thread = std::thread([this]() {
                conclusion_client.run();
                });

            m_consumerThread = std::thread([this]() {
                consumerLoop();
                });
        }
        catch (const std::exception& e) {
            logMessage(std::string("[Error] start(trade): ") + e.what() + "\n");
        }
    }

    void stop() {
        try {
            if (conclusion_thread.joinable()) {
                conclusion_client.stop();
                conclusion_thread.join();
            }
            m_stopRequested.store(true);
            if (m_consumerThread.joinable()) {
                m_consumerThread.join();
            }
        }
        catch (...) {}
    }

    // 최근 3초 데이터
    std::tuple<std::vector<TradeData>, std::vector<TradeData>, std::vector<TradeData>, std::vector<TradeData>, std::vector<TradeData>> getRecentTrades() const {
        std::lock_guard<std::mutex> lock(m_ringMutex);
        return { temporary_ringBuffer.getSnapshot(), shortsave_buy_ringBuffer.getSnapshot(), longsave_all_ringBuffer.getSnapshot(), midsave_buy_ringBuffer.getSnapshot(), midsave_sell_ringBuffer.getSnapshot() };
    }


private:

    void onMessage(client::message_ptr msg) {
        try {
            const std::string& payload = msg->get_payload();
            size_t len = payload.size();
            if (len > g_scratchBuffer.size() - simdjson::SIMDJSON_PADDING) {
                throw std::runtime_error("Message too large");
            }
            std::memcpy(g_scratchBuffer.data(), payload.data(), len);
            std::memset(g_scratchBuffer.data() + len, 0, simdjson::SIMDJSON_PADDING);

            static thread_local simdjson::ondemand::parser parser;
            simdjson::ondemand::document doc = parser.iterate(
                reinterpret_cast<const uint8_t*>(g_scratchBuffer.data()),
                len,
                g_scratchBuffer.size()
            );

            //{
            //        "e": "trade",         // Event type
            //        "E" : 123456789,       // Event time (밀리초 단위, UNIX timestamp)
            //        "s" : "BTCUSDT",       // Symbol (거래 페어)
            //        "t" : 12345,           // Trade ID (체결 ID)
            //        "p" : "0.001",         // Price (체결 가격)
            //        "q" : "100",           // Quantity (체결 수량)
            //        "b" : 88,              // Buyer order ID (매수 주문 ID)
            //        "a" : 50,              // Seller order ID (매도 주문 ID)
            //        "T" : 123456785,       // Trade time (체결 시간, 밀리초 단위, UNIX timestamp)
            //        "m" : true,            // Is the buyer the market maker? (매수자가 메이커인지 여부)
            //        "M" : true             // Ignore (무시)
            //}

            double  price = 0.0;
            double  quantity = 0.0;
            int64_t tms = 0;
            bool    isBuyerMaker = false; // m 필드를 저장할 변수

            auto pVal = doc["p"];
            if (!pVal.error()) {
                std::string_view pStr = std::string_view(pVal.get_string());
                std::from_chars(pStr.data(), pStr.data() + pStr.size(), price);
            }
            auto qVal = doc["q"];
            if (!qVal.error()) {
                std::string_view pStr = std::string_view(qVal.get_string());
                std::from_chars(pStr.data(), pStr.data() + pStr.size(), quantity);
            }
            auto tVal = doc["T"];
            if (!tVal.error()) {
                tms = tVal.get_int64();
            }
            auto mVal = doc["m"];
            if (!mVal.error()) {
                isBuyerMaker = mVal.get_bool(); // m 필드를 bool로 가져옴
            }

            TradeData td;
            td.price = price;
            td.quantity = quantity;
            td.tradetime = tms;
            td.side = isBuyerMaker;

            // 큐에 넣기
            m_tradeQueue.enqueue(td);
        }
        catch (const std::exception& e) {
            logMessage(std::string("[Error] onMessage: ") + e.what() + "\n");
        }
    }

    // 소비 스레드: bulk dequeue & ring buffer ===============================================================
    void consumerLoop() {
        std::vector<TradeData> batch;
        size_t batchSize = 1000;
        // 60,000 = 1m
        int64_t temp_time = 2000;
        int64_t long_time = 300000;
        int64_t mid_time = 60000;
        batch.reserve(batchSize);

        while (!m_stopRequested.load()) {
            batch.clear();
            m_tradeQueue.try_dequeue_bulk(std::back_inserter(batch), batchSize);

            if (!batch.empty()) {
                int64_t nowMs = std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::system_clock::now().time_since_epoch()).count();

                // 한번만 락 잡고 ring buffer 작업
                {
                    std::lock_guard<std::mutex> lock(m_ringMutex);
                    
                    for (auto& item : batch) {
                        temporary_ringBuffer.push_back(item);
                        longsave_all_ringBuffer.push_back(item);
                        if (item.side == false) {
                            shortsave_buy_ringBuffer.push_back(item);

                            midsave_buy_ringBuffer.push_back(item);
                        }
                        else {
                            midsave_sell_ringBuffer.push_back(item);
                        }
                        //else {
                        //    longsave_sell_ringBuffer.push_back(item);
                        //}
                    }
                    temporary_ringBuffer.discardOld(nowMs, temp_time);
                    longsave_all_ringBuffer.discardOld(nowMs, long_time);
                    shortsave_buy_ringBuffer.discardOld(nowMs, temp_time);
                    midsave_buy_ringBuffer.discardOld(nowMs, mid_time);
                    midsave_sell_ringBuffer.discardOld(nowMs, mid_time);
                    //longsave_sell_ringBuffer.discardOld(nowMs, long_time);

                }
            }
            else{
                std::this_thread::sleep_for(std::chrono::milliseconds(1));
            }
        }
    }


private:
    // WebSocket
    client                        conclusion_client;
    std::string                   m_uri;
    std::thread                   conclusion_thread;
    std::atomic<bool>             m_stopRequested{ false };

    // 소비
    std::thread                   m_consumerThread;

    // lock-free 큐
    TradeQueue                    m_tradeQueue;

    // Ring buffer for 3-second data
    mutable std::mutex            m_ringMutex;
    RingBuffer                    temporary_ringBuffer;


    RingBuffer                    shortsave_buy_ringBuffer;
    //RingBuffer                    longsave_sell_ringBuffer;
    RingBuffer                    longsave_all_ringBuffer;

    RingBuffer                    midsave_buy_ringBuffer;
    RingBuffer                    midsave_sell_ringBuffer;

    // simdjson scratch buffer
    static thread_local std::vector<char> g_scratchBuffer;
};

thread_local std::vector<char> reconquistaConclusion::g_scratchBuffer(
    65536 + simdjson::SIMDJSON_PADDING
);




using client = websocketpp::client<websocketpp::config::asio_tls_client>;

static constexpr double FEE_RATE = 0.0004;

struct BookEntry {
    double price;
    double volume;
};


// cURL RAII Helper ===============================================================
class CurlHandle {
public:
    CurlHandle() {
        m_curl = curl_easy_init();
        if (!m_curl) {
            throw std::runtime_error("Failed to init cURL handle");
        }
    }
    ~CurlHandle() {
        if (m_curl) {
            curl_easy_cleanup(m_curl);
            m_curl = nullptr;
        }
    }

    CurlHandle(const CurlHandle&) = delete;
    CurlHandle& operator=(const CurlHandle&) = delete;

    CurlHandle(CurlHandle&& other) noexcept : m_curl(other.m_curl) {
        other.m_curl = nullptr;
    }
    CurlHandle& operator=(CurlHandle&& other) noexcept {
        if (this != &other) {
            if (m_curl) {
                curl_easy_cleanup(m_curl);
            }
            m_curl = other.m_curl;
            other.m_curl = nullptr;
        }
        return *this;
    }
    CURL* get() { return m_curl; }

private:
    CURL* m_curl;
};

// cURL write callback ===================================================================

static size_t CurlWriteCallback(void* contents, size_t size, size_t nmemb, void* userp) {
    size_t totalSize = size * nmemb;
    if (!userp) return 0;
    std::string* buffer = static_cast<std::string*>(userp);
    buffer->append(static_cast<char*>(contents), totalSize);
    return totalSize;
}

// BinanceFuturesOrderBook ===============================================================
class BinanceFuturesOrderBook {
public:
    explicit BinanceFuturesOrderBook(const std::string& symbol, reconquistaConclusion& conclusion, int snapshotDepth = 1000)
        : m_symbol(symbol)
        , m_balance(1000000.0)
        , bestAsk_before(0.0)
        , m_entryPrice(0.0)
        , m_quantity(0.0)
        , m_snapshotDepth(snapshotDepth)
        , m_lastUpdateId(0)
        , m_conclusion(conclusion)
        , winrate(0)
        , tradecount(0)
    {
    }

    void fetchSnapshot(CurlHandle& curlHandle) {
        enforceSnapshotRateLimit();

        std::string url = "https://fapi.binance.com/fapi/v1/depth?symbol=" + m_symbol +
            "&limit=" + std::to_string(m_snapshotDepth);

        std::string responseBuffer;
        curl_easy_setopt(curlHandle.get(), CURLOPT_URL, url.c_str());
        curl_easy_setopt(curlHandle.get(), CURLOPT_WRITEFUNCTION, CurlWriteCallback);
        curl_easy_setopt(curlHandle.get(), CURLOPT_WRITEDATA, &responseBuffer);
        curl_easy_setopt(curlHandle.get(), CURLOPT_TIMEOUT, 5L); // 타임아웃 5초 등

        CURLcode res = curl_easy_perform(curlHandle.get());
        if (res != CURLE_OK) {
            throw std::runtime_error(
                "curl_easy_perform() failed: " + std::string(curl_easy_strerror(res)));
        }
        parseSnapshot(responseBuffer);
    }

    void applyDelta(const std::string& payload) {
        static thread_local simdjson::dom::parser s_wsParser;

        simdjson::dom::element doc;
        auto error = s_wsParser.parse(payload).get(doc);
        if (error) {
            std::cerr << "[WS] JSON parse error: " << simdjson::error_message(error) << std::endl;
            return;
        }

        // pu, u
        auto puRes = doc["pu"];
        auto uRes = doc["u"];
        if (puRes.error() || uRes.error()) {
            return;
        }

        int64_t pu = 0, u = 0;
        if (!getInt64Safely(puRes.value(), pu)) return;
        if (!getInt64Safely(uRes.value(), u))  return;

        std::unique_lock<std::mutex> lock(m_orderBookMutex);

        if (u <= m_lastUpdateId) {
            return;
        }
        if (pu > m_lastUpdateId) {
            std::cerr << "[WARN] Missing data. (pu=" << pu << ", local=" << m_lastUpdateId
                << ") => Resync..." << std::endl;
            lock.unlock();
            try {
                if (m_curlHandleForResync) {
                    fetchSnapshot(*m_curlHandleForResync);
                }
                else {
                    std::cerr << "[ERROR] No cURL handle for resync. Unable to fetch snapshot.\n";
                }
            }
            catch (const std::exception& e) {
                std::cerr << "[ERROR] Resync fetch snapshot failed: " << e.what() << std::endl;
            }
            return;
        }

        applyBidDelta(doc);
        applyAskDelta(doc);
        m_lastUpdateId = u;
    }


    // (3) OrderBook 출력
    void main_thread(int depth = 5) {

        std::lock_guard<std::mutex> lock(m_orderBookMutex);

        std::vector<std::pair<double, double>> avec(m_asks.begin(), m_asks.end());
        std::sort(avec.begin(), avec.end(), [](auto& l, auto& r) { return l.first < r.first; });
        std::vector<std::pair<double, double>> bvec(m_bids.begin(), m_bids.end());
        std::sort(bvec.begin(), bvec.end(), [](auto& l, auto& r) { return l.first > r.first; });

        auto [recentTrades, shortSaveTradesBuy, longSaveTradeAll, midBuy, midSell] = m_conclusion.getRecentTrades();


        // obi ========================================================
        constexpr size_t maxElements = 15;
        constexpr double weights[maxElements] = { 1, 1, 1, 1, 1, 0.6, 0.5, 0.4, 0.3, 0.2, 0.2, 0.2, 0.2, 0.2, 0.2};

        constexpr int SHIFT = 16;
        constexpr double SCALE = static_cast<double>(1 << SHIFT);

        long long bidAccum = 0;
        long long askAccum = 0;

        // 백터 크기가 무조건 같으면 그냥 하나로 묶어도 되긴 하는데, 혹시 앞으로 짤때 코인 이것저것 건드리면서 호가 주문 차이 큰 코인 만날 수도 있으니까 대비해서 그냥 두개로 분할
        for (size_t i = 0; i < bvec.size() && i < maxElements; ++i) {
            long long volFP = static_cast<long long>(bvec[i].second * SCALE + 0.5);
            long long wFP = static_cast<long long>(weights[i] * SCALE + 0.5);
            bidAccum += (volFP * wFP) >> SHIFT;
        }

        for (size_t i = 0; i < avec.size() && i < maxElements; ++i) {
            long long volFP = static_cast<long long>(avec[i].second * SCALE + 0.5);
            long long wFP = static_cast<long long>(weights[i] * SCALE + 0.5);
            askAccum += (volFP * wFP) >> SHIFT;
        }

        double weightedBidVolume = static_cast<double>(bidAccum) / SCALE;
        double weightedAskVolume = static_cast<double>(askAccum) / SCALE;

        double obi = weightedBidVolume / (weightedBidVolume + weightedAskVolume);
        // =============================================================



        // conclusion Volume ===========================================
        const int64_t window_size = 300; // 최근 5분(300초)
        const double delta_t = 2.0;      // 단기 시간 구간 (1초)

        double long_term_total_volume = std::accumulate(
            longSaveTradeAll.begin(), longSaveTradeAll.end(), 0.0,
            [](double total, const TradeData& trade) { return total + trade.quantity; });
        double recent_total_volume = std::accumulate(
            recentTrades.begin(), recentTrades.end(), 0.0,
            [](double total, const TradeData& trade) { return total + trade.quantity; });
        double average_volume = long_term_total_volume / static_cast<double>(window_size);
        double trade_intensity = recent_total_volume / (delta_t * average_volume);
        // =============================================================


        // buy 매수 주도 비율 계산 =======================================
        double total_short_buy = std::accumulate(
            shortSaveTradesBuy.begin(),
            shortSaveTradesBuy.end(),
            0.0,
            [](double sum, const auto& trade) { return sum + trade.quantity; });
        double total_long_buy = std::accumulate(
            recentTrades.begin(),
            recentTrades.end(),
            0.0,
            [](double sum, const auto& trade) { return sum + trade.quantity; });
        double Aggression = total_short_buy / total_long_buy;
        // =============================================================

        // 비축 물량 계산 ================================================
        double mid_term_buy_volume = std::accumulate(
            midBuy.begin(), midBuy.end(), 0.0,
            [](double total, const TradeData& trade) { return total + trade.quantity; });
        double mid_term_sell_volume = std::accumulate(
            midSell.begin(), midSell.end(), 0.0,
            [](double total, const TradeData& trade) { return total + trade.quantity; });
        double result_quantity = mid_term_buy_volume - mid_term_sell_volume;
        // =============================================================

        double  bestAsk = avec[0].first;
        double  bestBid = bvec[0].first;
        
        if (!g_havePosition) {

    }
    void setCurlHandleForResync(CurlHandle* ch) {
        m_curlHandleForResync = ch;
    }

//private:
//    torch::jit::script::Module model;

private:
    void parseSnapshot(const std::string& responseBuffer) {
        simdjson::dom::parser parser;
        simdjson::dom::element doc;
        auto error = parser.parse(responseBuffer).get(doc);
        if (error) {
            throw std::runtime_error(
                "Invalid or parse error in snapshot: " + std::string(simdjson::error_message(error)));
        }

        auto lidRes = doc["lastUpdateId"];
        if (lidRes.error()) {
            throw std::runtime_error("Invalid snapshot JSON: missing lastUpdateId");
        }
        int64_t lastUpdateId = 0;
        if (!getInt64Safely(lidRes.value(), lastUpdateId)) {
            throw std::runtime_error("Invalid type for lastUpdateId");
        }

        std::unordered_map<double, double> newBids;
        std::unordered_map<double, double> newAsks;

        parseOrderArray(doc["bids"], newBids);
        parseOrderArray(doc["asks"], newAsks);

        {
            std::lock_guard<std::mutex> lock(m_orderBookMutex);
            m_bids = std::move(newBids);
            m_asks = std::move(newAsks);
            m_lastUpdateId = lastUpdateId;
        }

        std::cout << "[REST] Snapshot fetched. lastUpdateId=" << lastUpdateId << std::endl;
    }

    void applyBidDelta(simdjson::dom::element& doc) {
        auto bRes = doc["b"];
        if (!bRes.error() && !bRes.get_array().error()) {
            auto bArr = bRes.get_array().value();
            for (auto bidVal : bArr) {
                if (bidVal.is_array()) {
                    auto arr = bidVal.get_array().value();
                    if (arr.size() >= 2) {
                        double price = std::stod(std::string(arr.at(0).get_string().value()));
                        double volume = std::stod(std::string(arr.at(1).get_string().value()));
                        if (volume == 0.0) {
                            m_bids.erase(price);
                        }
                        else {
                            m_bids[price] = volume;
                        }
                    }
                }
            }
        }
    }

    void applyAskDelta(simdjson::dom::element& doc) {
        auto aRes = doc["a"];
        if (!aRes.error() && !aRes.get_array().error()) {
            auto aArr = aRes.get_array().value();
            for (auto askVal : aArr) {
                if (askVal.is_array()) {
                    auto arr = askVal.get_array().value();
                    if (arr.size() >= 2) {
                        double price = std::stod(std::string(arr.at(0).get_string().value()));
                        double volume = std::stod(std::string(arr.at(1).get_string().value()));
                        if (volume == 0.0) {
                            m_asks.erase(price);
                        }
                        else {
                            m_asks[price] = volume;
                        }
                    }
                }
            }
        }
    }

    // 일반화 배열([[price,volume],[price,volume]...])parse
    static void parseOrderArray(const simdjson::dom::element& elem,
        std::unordered_map<double, double>& container)
    {
        if (!elem.get_array().error()) {
            auto arr = elem.get_array().value();
            for (auto item : arr) {
                if (!item.get_array().error()) {
                    auto innerArr = item.get_array().value();
                    if (innerArr.size() >= 2) {
                        double price = std::stod(std::string(innerArr.at(0).get_string().value()));
                        double volume = std::stod(std::string(innerArr.at(1).get_string().value()));
                        container[price] = volume;
                    }
                }
            }
        }
    }


    static bool getInt64Safely(const simdjson::dom::element& elem, int64_t& outVal) {
        if (elem.is<int64_t>()) {
            outVal = elem.get_int64();
            return true;
        }
        if (elem.is<double>()) {
            outVal = static_cast<int64_t>(elem.get_double());
            return true;
        }
        return false;
    }

    void enforceSnapshotRateLimit() {
        using Clock = std::chrono::steady_clock;
        auto now = Clock::now();

        while (!m_snapshotTimestamps.empty()) {
            auto oldest = m_snapshotTimestamps.front();
            auto diffSec = std::chrono::duration_cast<std::chrono::seconds>(now - oldest).count();
            if (diffSec > 60) {
                m_snapshotTimestamps.pop_front();
            }
            else {
                break;
            }
        }

        // 대체 왜 여기서 오류가 뜨냐
        if (m_snapshotTimestamps.size() >= 10) {
            auto oldest = m_snapshotTimestamps.front();
            auto diffSec = 60 - std::chrono::duration_cast<std::chrono::seconds>(now - oldest).count();
            if (diffSec > 0) {
                std::cout << "[RATE LIMIT] 10 snapshots in last 60s -> wait "
                    << diffSec << "s...\n";
                std::this_thread::sleep_for(std::chrono::seconds(diffSec));
            }

            now = Clock::now();
            while (!m_snapshotTimestamps.empty()) {
                auto oldest2 = m_snapshotTimestamps.front();
                auto diffSec2 = std::chrono::duration_cast<std::chrono::seconds>(now - oldest2).count();
                if (diffSec2 > 60) {
                    m_snapshotTimestamps.pop_front();
                }
                else {
                    break;
                }
            }
        }

        m_snapshotTimestamps.push_back(now);
    }

private:
    std::string m_symbol;
    int         m_snapshotDepth;

    std::unordered_map<double, double> m_bids;
    std::unordered_map<double, double> m_asks;

    double       m_balance;
    double       m_entryPrice;
    double       m_quantity;

    double       bestAsk_before;

    int          winrate;
    int          tradecount;
    int64_t          m_lastUpdateId;
    std::mutex       m_orderBookMutex;
    CurlHandle* m_curlHandleForResync = nullptr;

    reconquistaConclusion& m_conclusion;

    // "분당 10회" 제한
    std::deque<std::chrono::steady_clock::time_point> m_snapshotTimestamps;
};

// BinanceFuturesWsClient ===================================================

class BinanceFuturesWsClient {
public:
    using WssClient = websocketpp::client<websocketpp::config::asio_tls_client>;
    using context_ptr = websocketpp::lib::shared_ptr<ssl::context>;

    BinanceFuturesWsClient(const std::string& wsUrl, BinanceFuturesOrderBook& orderBook)
        : m_wsUrl(wsUrl)
        , m_orderBook(orderBook)
        , m_opened(false)
        , m_done(false)
    {
        m_client.clear_access_channels(websocketpp::log::alevel::all);
        m_client.clear_error_channels(websocketpp::log::elevel::all);

        m_client.init_asio();

        m_client.set_tls_init_handler([this](websocketpp::connection_hdl) -> context_ptr {
            auto ctx = websocketpp::lib::make_shared<ssl::context>(ssl::context::tlsv12);
            return ctx;
            });

        m_client.set_message_handler(
            [this](websocketpp::connection_hdl, WssClient::message_ptr msg) {
                if (msg->get_opcode() == websocketpp::frame::opcode::text) {
                    // depthUpdate => orderBook.applyDelta
                    m_orderBook.applyDelta(msg->get_payload());
                }
            }
        );


        m_client.set_open_handler([this](websocketpp::connection_hdl) {
            m_opened = true;
            std::cout << "[WS] Connected to " << m_wsUrl << std::endl;
            });
        m_client.set_close_handler([this](websocketpp::connection_hdl) {
            m_opened = false;
            std::cout << "[WS] Disconnected from " << m_wsUrl << std::endl;
            });
    }

    ~BinanceFuturesWsClient() {
        stop();
    }

    void start() {
        websocketpp::lib::error_code ec;
        auto conn = m_client.get_connection(m_wsUrl, ec);
        if (ec) {
            throw std::runtime_error("Could not create WS connection: " + ec.message());
        }
        m_hdl = conn->get_handle();
        m_client.connect(conn);

        m_thread.reset(new std::thread([this]() {
            try {
                m_client.run();
            }
            catch (const std::exception& e) {
                std::cerr << "[WS] run() exception: " << e.what() << std::endl;
            }
            }));
    }

    void stop() {
        if (m_done.exchange(true)) {
            return;
        }
        if (m_opened) {
            websocketpp::lib::error_code ec;
            m_client.close(m_hdl, websocketpp::close::status::normal, "", ec);
            if (ec) {
                std::cerr << "[WS] Close error: " << ec.message() << std::endl;
            }
        }
        if (m_thread && m_thread->joinable()) {
            m_thread->join();
        }
    }

private:
    std::string              m_wsUrl;
    BinanceFuturesOrderBook& m_orderBook;

    WssClient                m_client;
    websocketpp::connection_hdl m_hdl;

    bool                     m_opened;
    std::atomic<bool>        m_done;
    std::unique_ptr<std::thread> m_thread;
};

void countdown(int minutes) {
    int seconds = minutes * 60;
    while (seconds >= 0) {
        int mins = seconds / 60;
        int secs = seconds % 60;

        std::cout << "Time remaining: "
            << mins << " minutes "
            << secs << " seconds\r" << std::flush;

        std::this_thread::sleep_for(std::chrono::seconds(1));
        --seconds;
    }
    std::cout << "\nCountdown complete!\n";
}



int main() {

    //system("mode con cols=48 lines=30 | title Reconquista"); 


    timeBeginPeriod(1);
    logInit();
    std::cout << std::fixed << std::setprecision(5);
    if (!SetPriorityClass(GetCurrentProcess(), REALTIME_PRIORITY_CLASS)) {
        logMessage("[Warn] Failed to set REALTIME_PRIORITY_CLASS\n");
    }
    if (!SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL)) {
        logMessage("[Warn] Failed to set THREAD_PRIORITY_TIME_CRITICAL\n");
    }
    DWORD_PTR affinityMask = 1ull << 0; // CPU0
    if (!SetProcessAffinityMask(GetCurrentProcess(), affinityMask)) {
        logMessage("[Warn] Failed to set CPU Affinity\n");
    }

    curl_global_init(CURL_GLOBAL_DEFAULT);
    CurlHandle curlHandle;

    std::string symbol = "btcusdt";
    int         size = 20;

    reconquistaConclusion conclusion(symbol);
    conclusion.start();

    BinanceFuturesOrderBook orderBook(symbol, conclusion);
    orderBook.setCurlHandleForResync(&curlHandle);
    orderBook.fetchSnapshot(curlHandle);
    orderBook.main_thread(size);
    std::string wsSymbol = symbol;
    for (auto& c : wsSymbol) c = ::tolower(c);
    std::string wsUrl = "wss://fstream.binance.com/ws/" + wsSymbol + "@depth";
    BinanceFuturesWsClient wsClient(wsUrl, orderBook);
    wsClient.start();

    int countdownMinutes = 5; // 5분 설정
    countdown(countdownMinutes);
    system("cls");
    while (true) {
        orderBook.main_thread(size);
        std::this_thread::sleep_for(std::chrono::milliseconds(200));
    }


    logMessage("Press Enter to stop...\n");
    std::cin.get();

    wsClient.stop();
    curl_global_cleanup();
    conclusion.stop(); 

    logStop();
    timeEndPeriod(1);

    return 0;
}
