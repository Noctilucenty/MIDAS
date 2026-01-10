#include <iostream>
#include <string>
#include <chrono>
#include <sstream>
#include <iomanip>
#include <curl/curl.h>
#include <openssl/hmac.h>
#include <openssl/evp.h>
#include <thread>

static size_t WriteCallback(void* contents, size_t size, size_t nmemb, void* userp) {
    size_t totalSize = size * nmemb;
    std::string* str = static_cast<std::string*>(userp);
    str->append(static_cast<char*>(contents), totalSize);
    return totalSize;
}

int main() {


    const std::string apiKey = "mkxeSk1ZMLaGYzEEXW11jwYpiknZycVXIhM7Tw5Tk1syTjytv1iadW3QQ84ruIU0";
    const std::string secretKey = "NX4VmRWKs6OB1iiKKcRXA9ZgsPoTUpT5bKpm0tXaLa0lJeGB3UNe5kjZinWpBJRh";


    // const std::string symbol = "WLDUSDT";
    // const std::string side = "BUY";
    // const std::string type = "MARKET";
    // const double quantity = 3;

    // const std::string symbol = "WLDUSDT";
    // const std::string side = "BUY";
    // const std::string type = "LIMIT";
    // const double quantity = 3;

    // const std::string symbol = "WLDUSDT";
    // const std::string side = "SELL";
    // const std::string type = "MARKET";
    // const double quantity = 3;

    long long timestamp = std::chrono::duration_cast<std::chrono::milliseconds>(
        std::chrono::system_clock::now().time_since_epoch()
    ).count();
    long long recvWindow = 1000;

    std::ostringstream oss;
    oss << "symbol=" << symbol
        << "&side=" << side
        << "&type=" << type
        << "&quantity=" << quantity
        << "&timestamp=" << timestamp
        << "&recvWindow=" << recvWindow;
    std::string queryString = oss.str();

    unsigned char* digest = HMAC(
        EVP_sha256(),
        secretKey.c_str(), secretKey.size(),
        reinterpret_cast<const unsigned char*>(queryString.c_str()), queryString.size(),
        nullptr, nullptr
    );
    if (!digest) {
        std::cerr << "HMAC failed.\n";
        return -1;
    }
    std::ostringstream signOss;
    signOss << std::hex << std::setfill('0');
    for (int i = 0; i < EVP_MD_size(EVP_sha256()); ++i) {
        signOss << std::setw(2) << (unsigned int)digest[i];
    }
    queryString += "&signature=" + signOss.str();

    CURL* curl = nullptr;
    CURLcode res;
    curl_global_init(CURL_GLOBAL_ALL);
    curl = curl_easy_init();
    if (!curl) {
        std::cerr << "curl init failed.\n";
        return -1;
    }

    std::string url = "https://fapi.binance.com/fapi/v1/order";
    curl_easy_setopt(curl, CURLOPT_URL, url.c_str());
    curl_easy_setopt(curl, CURLOPT_POST, 1L);

    struct curl_slist* headers = nullptr;
    headers = curl_slist_append(headers, ("X-MBX-APIKEY: " + apiKey).c_str());
    headers = curl_slist_append(headers, "Content-Type: application/x-www-form-urlencoded");
    curl_easy_setopt(curl, CURLOPT_HTTPHEADER, headers);

    curl_easy_setopt(curl, CURLOPT_TCP_KEEPALIVE, 1L);
    curl_easy_setopt(curl, CURLOPT_TCP_NODELAY, 1L);
    curl_easy_setopt(curl, CURLOPT_DNS_CACHE_TIMEOUT, 600L);
    // curl_easy_setopt(curl, CURLOPT_TCP_KEEPIDLE, 30L);
    // curl_easy_setopt(curl, CURLOPT_TCP_KEEPINTVL, 10L);

    curl_easy_setopt(curl, CURLOPT_POSTFIELDS, queryString.c_str());


    std::cout << "6" << std::endl;


    std::string response;
    curl_easy_setopt(curl, CURLOPT_WRITEFUNCTION, WriteCallback);
    curl_easy_setopt(curl, CURLOPT_WRITEDATA, &response);


    res = curl_easy_perform(curl);
    if (res != CURLE_OK) {
        std::cerr << "curl error: " << curl_easy_strerror(res) << std::endl;
    }
    else {
        long httpCode = 0;
        curl_easy_getinfo(curl, CURLINFO_RESPONSE_CODE, &httpCode);
        std::cout << "[HTTP " << httpCode << "] " << response << std::endl;
    }

    curl_slist_free_all(headers);
    curl_easy_cleanup(curl);
    curl_global_cleanup();

    return 0;
}
