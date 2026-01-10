#define _CRT_SECURE_NO_WARNINGS
#include <H5Cpp.h>
#include <iostream>
#include <vector>
#include <chrono>
#include <thread>
#include <iomanip>
#include <algorithm>
#include <string>
#include <windows.h>
#include <ta_libc.h>
#include <numeric>
#include <unordered_map>
#include <cstdlib>
#include <map>
#include <span>


// 2024 - 11 - 28 20:21 : 00
long long start_time_set(int a = 2019, int b = 10, int c = 1, int d = 20, int e = 21) {
    std::tm specific_time = {};
    specific_time.tm_year = a - 1900;  // 연도 설정
    specific_time.tm_mon = b - 1;      // 월 설정 (0-11)
    specific_time.tm_mday = c;         // 일 설정
    specific_time.tm_hour = d;         // 시간 설정
    specific_time.tm_min = e;          // 분 설정
    specific_time.tm_sec = 0;          // 초 설정


    std::time_t time_t_value = std::mktime(&specific_time);

    if (time_t_value == -1) {
        std::cerr << "Error: Time conversion failed." << std::endl;
        return -1;
    }

    long long timestamp_seconds = static_cast<long long>(time_t_value);
    long long timestamp_microseconds = timestamp_seconds * (static_cast<long long>(1e6) * 1000);

    return timestamp_microseconds;
}

// map -> unordered_map로 변경 (속도 향상에 약간 유리)
std::unordered_map<std::string, std::unordered_map<int, std::vector<double>>>
getDatasetMap(hsize_t cdl, hsize_t ran = 0)
{
    std::unordered_map<std::string, std::unordered_map<int, std::vector<double>>> datasetMap;

    const char* filename = "D:\\workspace\\goldi_locks\\st_tester\\dataset\\Backtest_Market_data.h5";
    std::string group_name = "/interval_1m";
    H5::H5File file(filename, H5F_ACC_RDONLY);
    H5::Group group = file.openGroup(group_name);
    hsize_t num_objs = group.getNumObjs();
    std::vector<std::string> dataset_names;
    if (ran == 0) {
        ran = num_objs;
    }

    for (hsize_t i = 0; i < ran; ++i) {
        std::string obj_name = group.getObjnameByIdx(i);
        H5G_obj_t obj_type = group.getObjTypeByIdx(i);
        if (obj_type == H5G_DATASET) {
            dataset_names.push_back(obj_name);
        }
    }
    // 위에 자동적으로 세팅해서 사용하거나,

    //아래에
    dataset_names = { "BTCUSDT","XRPUSDT","ETHUSDT","BNBUSDT","SOLUSDT","DOGEUSDT","SUIUSDT","WIFUSDT","1000PEPEUSDT","FETUSDT","SEIUSDT"};


    // ---- progress bar ----
    // const int LEN = 20;
    // float tick = (float)100 / LEN;
    // std::cout << tick << "% per bar 1 print\n\n";
    // int count = 0;
    // int MAX = (int)dataset_names.size();

    for (size_t idx = 0; idx < dataset_names.size(); ++idx) {
        const auto& name = dataset_names[idx];
        
        H5::DataSet dataset = file.openDataSet("/interval_1m/" + name); // 여기서 이름 설정 "/interval_1m/" + name

        H5::DataSpace dataspace = dataset.getSpace();
        hsize_t dims[2] = {};
        dataspace.getSimpleExtentDims(dims, nullptr);

        // cdl*11 이상 데이터가 있을 때만 로딩
        if (cdl * 11 < dims[0]) {
            std::vector<double> data(dims[0] * dims[1]);
            dataset.read(data.data(), H5::PredType::NATIVE_DOUBLE);
            size_t numEntries = data.size() / 10;

            // datasetMap[name]의 index별로 push_back
            auto& mapRef = datasetMap[name];
            // 미리 reserve해서 push_back 성능 최적화
            mapRef[0].reserve(numEntries); // open time
            mapRef[1].reserve(numEntries); // open price
            mapRef[2].reserve(numEntries); // high price
            mapRef[3].reserve(numEntries); // low price
            mapRef[4].reserve(numEntries); // close price
            mapRef[5].reserve(numEntries); // volume

            for (size_t i = 0; i < numEntries; i++) {
                mapRef[0].push_back(data[i * 10]);      // opentime
                mapRef[1].push_back(data[i * 10 + 1]);  // open
                mapRef[2].push_back(data[i * 10 + 2]);  // high
                mapRef[3].push_back(data[i * 10 + 3]);  // low
                mapRef[4].push_back(data[i * 10 + 4]);  // close
                mapRef[5].push_back(data[i * 10 + 5]);  // volume
            }
        }

        // 진행 상황 표시 - 너무 자주 호출하면 성능 떨어짐
        // std::cout << "\rLoading " << (idx + 1) << "/" << dataset_names.size() << " [" << name << "]";
        // std::cout.flush();

        // count++;
        // Sleep(5); // Sleep 제거(최적화)
    }

    // std::cout << "\nDone loading!\n\n";
    H5close();
    return datasetMap;
}

inline void time_printer(long long time_parameter, short UTC_C) {
    int a = static_cast<int>(time_parameter / (static_cast<long long>(1e6) * 1000));
    std::time_t unix_timestamp = a;
    unix_timestamp += UTC_C * 3600; // 분까지는 고려 안 하고 3600곱
    std::tm* date = std::gmtime(&unix_timestamp);
    std::cout << std::put_time(date, "%Y-%m-%d %H:%M:%S") << std::endl;
}







// 헷징 숏 주문
inline std::pair<std::unordered_map<std::string, std::vector<double>>, std::unordered_map<std::string, std::vector<double>>> Hed_BUY(std::unordered_map<std::string, std::vector<double>>& hed_wallet, std::unordered_map<std::string, std::vector<double>>& wallet, hsize_t buy_amount, double Market_Price, const std::string& sym, long long stt, short UTC_C)
{
    
    // [0] 매수방법, [1] 매수가, [2] 수량, [3] 상태(추가매수 횟수), [4] defend, [5] defend_price, [6] delay_time

    if (hed_wallet.find(sym) != hed_wallet.end()) {
        
        if (hed_wallet.find(sym) != hed_wallet.end() && hed_wallet[sym][0] == 0) {
            hed_wallet[sym][3] += 1;
            buy_amount = buy_amount * (hsize_t)hed_wallet[sym][3];
            double quantity = buy_amount / Market_Price;
            hed_wallet[sym][1] = ((hed_wallet[sym][1] * hed_wallet[sym][2]) + (Market_Price * quantity)) / (hed_wallet[sym][2] + quantity);
            hed_wallet[sym][2] += quantity;
            std::cout << sym << " hedging short additional purchased ";

            std::cout << std::setprecision(7) << wallet["balance"][0] << std::endl;
            time_printer(stt, UTC_C);
            std::cout << std::endl;
        }
        else if (hed_wallet.find(sym) != hed_wallet.end() && hed_wallet[sym][0] == 1) {
            double quantity = buy_amount / Market_Price;
            hed_wallet[sym][1] = ((hed_wallet[sym][1] * hed_wallet[sym][2]) + (Market_Price * quantity)) / (hed_wallet[sym][2] + quantity);
            hed_wallet[sym][2] += quantity;
        }
        else {
            double quantity = buy_amount / Market_Price;
            hed_wallet.erase(sym);
            hed_wallet[sym] = { 0, Market_Price, quantity, 1, 0, 0, 3 };
            std::cout << sym << " hedging short buy Complete " << std::setprecision(7) << Market_Price;


            std::cout << std::setprecision(7) << wallet["balance"][0] << std::endl;
            time_printer(stt, UTC_C);
            std::cout << std::endl;
        }
        wallet["balance"][0] -= buy_amount * 0.0002;
    }
    return { wallet, hed_wallet };
}
inline std::pair<std::unordered_map<std::string, std::vector<double>>, std::unordered_map<std::string, std::vector<double>>> Hed_SELL(std::unordered_map<std::string, std::vector<double>>& hed_wallet, std::unordered_map<std::string, std::vector<double>>& wallet, double Market_Price, const std::string& sym, long long stt, short UTC_C)
{

    if (hed_wallet.find(sym) != hed_wallet.end()) {
        int hed_state = hed_wallet[sym][0];
        if (hed_state == 0) {
            double bought_price = hed_wallet[sym][1];
            double quantity = hed_wallet[sym][2];
            double sell_amount = Market_Price * quantity;
            double fee = sell_amount * 0.0004;
            double PNL = (bought_price - Market_Price) * quantity - fee;
            wallet["balance"][0] += PNL;

            if (PNL > 0)
                std::cout << sym << "\033[32mShort Take Profit Sell Completed\033[0m" << std::setprecision(7) << wallet["balance"][0] << std::endl;
            else
                std::cout << sym << "\033[31mShort Stop Loss Sell Completed\033[0m" << std::setprecision(7) << wallet["balance"][0] << std::endl;

            time_printer(stt, UTC_C);
            std::cout << std::endl;
            hed_wallet.erase(sym);
        }
        else {
            hed_wallet.erase(sym);
        }
    }
    return { wallet, hed_wallet };
}







// 일반 롱 주문
inline std::unordered_map<std::string, std::vector<double>> BUY(std::unordered_map<std::string, std::vector<double>>& wallet, hsize_t buy_amount, double Market_Price, const std::string& sym, long long stt, short UTC_C)
{
    // [0] 매수방법, [1] 매수가, [2] 수량, [3] 상태(추가매수 횟수), [4] defend, [5] defend_price, [6] delay_time
    if (wallet.find(sym) != wallet.end() && wallet[sym][0] == 0) {
        wallet[sym][3] += 1;
        buy_amount = buy_amount * (hsize_t)wallet[sym][3];
        double quantity = buy_amount / Market_Price;
        wallet[sym][1] = ((wallet[sym][1] * wallet[sym][2]) + (Market_Price * quantity)) / (wallet[sym][2] + quantity);
        wallet[sym][2] += quantity;
        std::cout << sym << " additional purchased ";

        std::cout << std::setprecision(7) << wallet["balance"][0] << std::endl;
        time_printer(stt, UTC_C);
        std::cout << std::endl;
    }
    else if (wallet.find(sym) != wallet.end() && wallet[sym][0] == 1) {
        double quantity = buy_amount / Market_Price;
        wallet[sym][1] = ((wallet[sym][1] * wallet[sym][2]) + (Market_Price * quantity)) / (wallet[sym][2] + quantity);
        wallet[sym][2] += quantity;
    }
    else {
        double quantity = buy_amount / Market_Price;
        wallet[sym] = { 1, Market_Price, quantity, 1, 0, 0, 5};
        std::cout << sym << " buy Complete ";


        std::cout << std::setprecision(7) << wallet["balance"][0] << std::endl;
        time_printer(stt, UTC_C);
        std::cout << std::endl;
    }
    wallet["balance"][0] -= buy_amount * 0.0002;

    return wallet;
}

inline std::unordered_map<std::string, std::vector<double>> SELL(std::unordered_map<std::string, std::vector<double>>& wallet, double Market_Price, const std::string& sym, long long stt, short UTC_C)
{
    double bought_price = wallet[sym][1];
    double quantity = wallet[sym][2];
    double sell_amount = Market_Price * quantity;
    double fee = sell_amount * 0.0004;
    double PNL = (Market_Price - bought_price) * quantity - fee;
    wallet["balance"][0] += PNL;

    if (PNL > 0)
        std::cout << sym << "\033[32mTake Profit Sell Completed\033[0m" << std::setprecision(7) << wallet["balance"][0] << std::endl;
    else
        std::cout << sym << "\033[31mStop Loss Sell Completed\033[0m" << std::setprecision(7) << wallet["balance"][0] << std::endl;

    time_printer(stt, UTC_C);
    std::cout << std::endl;
    wallet.erase(sym);
    return wallet;

}








struct TechnicalIndicators {
    std::vector<double> mfi;
    std::vector<double> adx;
    std::vector<double> SAR_1s;
    std::vector<double> SAR_1h;
    std::vector<double> SlowK;
    std::vector<double> SlowD;
    std::vector<double> UpperBand;
    std::vector<double> MiddleBand;
    std::vector<double> LowerBand;
    double VWAP;
    TechnicalIndicators() : VWAP(0.0) {}
};

TechnicalIndicators calculateTechnicalIndicators(
    const std::span<double>& H_p,
    const std::span<double>& L_p,
    const std::span<double>& C_p,
    const std::span<double>& V_p,
    size_t cdl);

int main() {

    std::unordered_map<std::string, int> activated_sym;


    std::unordered_map<std::string, std::vector<double>> wallet;
    std::unordered_map<std::string, std::vector<double>> hed_wallet;
    wallet["balance"] = { 1000000 }; // 백테스트 초기 자금
    hsize_t buy_amount = 20000;
    hsize_t add_buy_amount = 70000;
    long long stt = start_time_set();
    hsize_t cdl = 1000; // 지표에 사용될 캔들 수
    hsize_t ran = 11; // 티커
    short UTC_C = 9;
    int add_count = 3; // 추가 매수 최대 횟수


    // 유예 시간
    double time_delay_nb = 10; // 10분 후 초기화
    int delay_time_s = 10; // 추가매수 딜레이 타임


    auto datasetMap = getDatasetMap(cdl, ran);
    double last_time = 0.0;

    // 햇징용 알고리즘 하이퍼파라미터
    int hed_cdl = 20;
    int search_start = cdl - hed_cdl;
    int search_length = hed_cdl;  // 20분





    // start/end time 체크
    if (!datasetMap.empty()) {
        auto outer_it = datasetMap.begin();
        const std::string& first_key = outer_it->first;
        const auto& inner_map = outer_it->second;
        if (!inner_map.empty()) {
            const auto& vec = inner_map.at(0);
            if (!vec.empty()) {
                last_time = vec.back();
                 std::cout << "start time: ";
                 time_printer(stt, UTC_C);
                 std::cout << "end time: ";
                 time_printer((long long)last_time, UTC_C);

                 for (int i = 2; i > 0; --i) {
                     std::cout << "\rTime Remaining Until Start: " << i << std::flush;
                     std::this_thread::sleep_for(std::chrono::seconds(1));
                 }
                 #ifdef _WIN32
                     std::system("cls");
                 #else
                     std::system("clear");
                 #endif

            }
            else {
                std::cout << "no.0 index vector is '" << first_key << "' empty" << std::endl;
                return 0;
            }
        }
        else {
            std::cout << "time data index is empty '" << first_key << "'." << std::endl;
            return 0;
        }
    }
    else {
        std::cout << "dataset is empty." << std::endl;
        return 0;
    }

    // 메인 백테스트 루프
    while (true) {
        if (stt >= last_time) {
            std::cout << "backtesting ended" << std::endl;
            break;
        }

        // 종목별로 매 분봉 시뮬레이션
        for (auto& [sym, dataMatrix] : datasetMap) {
            if (activated_sym.find(sym) != activated_sym.end()) {
                long long range = activated_sym[sym];

                // range-cdl 음수가 되지 않도록 체크
                if (range < cdl) continue;

                // 0: opentime, 1: open, 2: high, 3: low, 4: close, 5: volume
                std::span<double> open(dataMatrix[1].data() + (range - cdl), cdl);
                std::span<double> high(dataMatrix[2].data() + (range - cdl), cdl);
                std::span<double> low(dataMatrix[3].data() + (range - cdl), cdl);
                std::span<double> close(dataMatrix[4].data() + (range - cdl), cdl);
                std::span<double> vol(dataMatrix[5].data() + (range - cdl), cdl);

                double Market_Price = close[close.size() - 1];


                // ===================================================================================================== 매수 매도 섹터
                // 이미 보유 중이라면 추가 매수 or 청산
                if (wallet.find(sym) != wallet.end()) {
                    
                    TechnicalIndicators indicators = calculateTechnicalIndicators(high, low, close, vol, cdl);

                    double swit = wallet[sym][0];

                    // 2차 심사 매수 
                    if (swit == -1) {
                        double time_le = wallet[sym][1];
                        
                        if (time_le) {
                            if (indicators.adx[indicators.adx.size() - 1] > 40){
                                wallet[sym][0] = -2; // 분할 매수 단계로 진입 2,3,4,5,6
                                double open_price_b = open[open.size() - 1];
                                double low_price_b = low[low.size() - 1];

                                if (low_price_b < Market_Price) {
                                    wallet[sym][2] = Market_Price;
                                    wallet[sym][3] = low_price_b + (Market_Price - low_price_b) * 0.6;
                                    wallet[sym][4] = low_price_b + (Market_Price - low_price_b) * 0.3;
                                    wallet[sym][5] = low_price_b + (Market_Price - low_price_b) * 0.1;
                                    wallet[sym][6] = low_price_b * 0.995;
                                }
                                else {
                                    wallet[sym][2] = low_price_b;
                                    wallet[sym][3] = low_price_b * 0.997;
                                    wallet[sym][4] = low_price_b * 0.995;
                                    wallet[sym][5] = low_price_b * 0.993;
                                    wallet[sym][6] = low_price_b * 0.992;
                                }

                                wallet[sym][1] = 10;
                            }
                            else {
                                wallet[sym][1] -= 1;
                            }

                        }
                        else {
                                wallet.erase(sym);
                        }
                    }
                    else if (swit == -2) {
                        double time_le = wallet[sym][1];
                        if (time_le) {
                            std::vector<double> prices = { wallet[sym][2], wallet[sym][3], wallet[sym][4], wallet[sym][5], wallet[sym][6] };

                            int sep_buy = prices.size();
                            double increment_amount = buy_amount / sep_buy;  // 각 매수 금액을 동일하게 나눔

                            if (prices[0] > Market_Price) {
                                wallet.erase(sym);
                                for (int i = 0; i < prices.size(); i++) {
                                    if (prices[i] > Market_Price) {
                                        double buy_amount_for_this_step = increment_amount * (i + 1);  // 반복마다 증가하는 매수 금액
                                        wallet = BUY(wallet, buy_amount_for_this_step, prices[i], sym, stt, UTC_C);
                                    }
                                }

                                std::span<double> search_range = low.subspan(search_start, std::min<size_t>(search_length, low.size() - search_start));


                                double min_value = search_range[0];
                                for (double value : search_range) {
                                    if (value < min_value) {
                                        min_value = value;
                                    }
                                }

                                ////////////////////////////////////////////////////////////////// 가격 차이가 1프로 내라면 1프로 밖에 숏 세팅
                                if (min_value > Market_Price * 0.99) {
                                    min_value = min_value * 0.995;
                                }


                                
                                hed_wallet[sym] = { -1, min_value };
                                wallet[sym][0] = 0;
                            }
                            else {
                                wallet[sym][1] -= 1;
                            }
                        }
                        else {
                            wallet.erase(sym);
                        }

                    }
                    // 추가매수 또는 손절 또는 익절 매도 기능
                    else {


                        double bought_price = wallet[sym][1];
                        int count = wallet[sym][3];
                        int delay_time = wallet[sym][6];
                        


                        if (delay_time) {
                            if (wallet.find(sym) != wallet.end()) {
                                wallet[sym][6] -= 1;
                            }
                        }


                        

                        // 헷징 시스템
                        if (hed_wallet.find(sym) != hed_wallet.end()) {
                            int hed_state = hed_wallet[sym][0];
                            if (hed_state == -1) {
                                double hed_price = hed_wallet[sym][1];
                                if (hed_price > Market_Price) {
                                    std::tie(wallet, hed_wallet) = Hed_BUY(hed_wallet, wallet, buy_amount, hed_price, sym, stt, UTC_C);
                                }
                            }
                            
                            if (hed_state == 1 || hed_state == 0) { // hed_state == 0 || 
                                int hed_delay_time = hed_wallet[sym][6];
                                if (hed_delay_time) {
                                    hed_wallet[sym][6] -= 1;
                                }
                                if (!hed_delay_time) {
                                    double hed_price = hed_wallet[sym][1];
                                    if (indicators.adx[indicators.adx.size() - 1] > 40 && indicators.mfi[indicators.mfi.size() - 1] < 5 && hed_price * 0.99 > Market_Price && hed_state == 1) {
                                        std::cout << "숏 매도 신호 생" << std::endl;
                                        std::tie(wallet, hed_wallet) = Hed_SELL(hed_wallet, wallet, Market_Price, sym, stt, UTC_C);
                                    }
                                    if (hed_price * 1.005 < Market_Price && hed_state == 0) { // stop limit order
                                        std::tie(wallet, hed_wallet) = Hed_SELL(hed_wallet, wallet, hed_price * 1.005, sym, stt, UTC_C);
                                        /*hed_wallet[sym] = { -1, min_value };*/
                                    }
                                }
                            }
                        }



                        if (indicators.adx[indicators.adx.size() - 1] > 30 && indicators.mfi[indicators.mfi.size() - 1] < 10 && count < add_count) { // 추가매수 하락시에
                            if (!delay_time) {
                                double add_amount = (add_buy_amount / add_count) * count;
                                wallet = BUY(wallet, add_amount, Market_Price, sym, stt, UTC_C);
                                std::tie(wallet, hed_wallet) = Hed_BUY(hed_wallet, wallet, add_buy_amount, Market_Price, sym, stt, UTC_C);
                                wallet[sym][6] = delay_time_s;
                            }
                        }
                        else if ((indicators.mfi[indicators.mfi.size() - 1] > indicators.mfi[indicators.mfi.size() - 2] && indicators.mfi[indicators.mfi.size() - 2] > indicators.mfi[indicators.mfi.size() - 3]&& indicators.mfi[indicators.mfi.size() - 3] > indicators.mfi[indicators.mfi.size() - 4]) && indicators.adx[indicators.adx.size() - 1] > 40 && 40 > indicators.mfi[indicators.mfi.size() - 1] > 20 && count < add_count) { // 추가매수 상승다이버전스
                            double low_price_2 = low[low.size() - 2];
                            if (Market_Price < low_price_2) {
                                if (!delay_time) {
                                    wallet = BUY(wallet, add_buy_amount, Market_Price, sym, stt, UTC_C);
                                    std::tie(wallet, hed_wallet) = Hed_BUY(hed_wallet, wallet, add_buy_amount, Market_Price, sym, stt, UTC_C);
                                    wallet[sym][6] = delay_time_s;
                                }
                            }   
                        }
                        else{
                            int defend_state = wallet[sym][4];

                            if (defend_state == 0) {
                                if (bought_price * 1.004 <= Market_Price) {
                                    wallet[sym][4] = 1;
                                    wallet[sym][5] = bought_price * 1.002; // 만약에 올랐다가, 0.1프로 하락시에, 매도하도록 구성.
                                    double ppp = wallet[sym][5];
                                }
                                else if (indicators.SAR_1s[indicators.SAR_1s.size() - 1] > Market_Price && count >= add_count) {
                                    wallet = SELL(wallet, Market_Price, sym, stt, UTC_C);
                                    std::tie(wallet, hed_wallet) = Hed_SELL(hed_wallet, wallet, Market_Price, sym, stt, UTC_C);
                                }
                            }
                            else {
                                double defend_price = wallet[sym][5];
                                if (defend_price > Market_Price) { // 방어 손절매도에서 지정가 주문이 발생한것을 구현하기 위해서 마켓 프라이스를 조정
                                    double Market_Price_d = wallet[sym][5];
                                    std::cout << "방어 모드 가동" << std::endl;
                                    wallet = SELL(wallet, Market_Price_d, sym, stt, UTC_C);
                                    std::tie(wallet, hed_wallet) = Hed_SELL(hed_wallet, wallet, Market_Price, sym, stt, UTC_C);
                                }
                                else if ((bought_price * 1.05 <= Market_Price) || (indicators.SAR_1s[indicators.SAR_1s.size() - 1] > Market_Price && (bought_price * 1.03 <= Market_Price)) || (bought_price * 1.01 <= Market_Price && indicators.mfi[indicators.mfi.size() - 1] > 80)) { //  || (bought_price * 1.003 <= Market_Price && indicators.mfi[indicators.mfi.size() - 1] > 80)
                                    wallet = SELL(wallet, Market_Price, sym, stt, UTC_C);
                                    std::tie(wallet, hed_wallet) = Hed_SELL(hed_wallet, wallet, Market_Price, sym, stt, UTC_C);
                                }
                            }
                        }
                    }
                }
                // 아직 미보유 상태라면 매수 로직
                else {

                    TechnicalIndicators indicators = calculateTechnicalIndicators(high, low, close, vol, cdl);
                    

                    // 1차 심사 매수
                    if (indicators.mfi[indicators.mfi.size() - 1] < 13
                        ) {
                        wallet[sym] = { -1 , time_delay_nb ,0 ,0 ,0 ,0 ,0}; // 분할 매수 가격 5개

                    }

                    //if (open[open.size() - 3] > close[close.size() - 3] &&
                    //    open[open.size() - 2] > close[close.size() - 2] &&
                    //    open[open.size() - 2] > Market_Price
                    //    ) {

                    //    TechnicalIndicators indicators = calculateTechnicalIndicators(high, low, close, vol, cdl);

                    //    size_t count = std::min<size_t>(30, high.size());

                    //    double max_p = high[high.size() - count];
                    //    for (size_t i = high.size() - count + 1; i < high.size(); ++i) {
                    //        if (high[i] > max_p) {
                    //            max_p = high[i];
                    //        }
                    //    }
                    //    if (indicators.SlowK[indicators.SlowK.size() - 4] < 4 &&
                    //        indicators.SlowD[indicators.SlowD.size() - 4] < 4 &&
                    //        indicators.SlowK[indicators.SlowK.size() - 3] < 2 &&
                    //        indicators.SlowD[indicators.SlowD.size() - 3] < 2 &&
                    //        indicators.SlowK[indicators.SlowK.size() - 2] < 4 &&
                    //        indicators.SlowD[indicators.SlowD.size() - 2] < 4 &&
                    //        indicators.SlowK[indicators.SlowK.size() - 1] > 2 &&
                    //        indicators.SlowK[indicators.SlowK.size() - 1] > indicators.SlowD[indicators.SlowD.size() - 1] &&

                    //        //indicators.VWAP > Market_Price &&

                    //        ((max_p - close[close.size() - 2]) / max_p * 100) > 1.6 

                    //        //indicators.SAR_1s[indicators.SAR_1s.size() - 1] < Market_Price
                    //        ) {

                    //        wallet = BUY(wallet, buy_amount, Market_Price, sym, stt, UTC_C);

                    //    }
                    //}
                } // ============================================================================================================================================================

                activated_sym[sym] = range + 1;
            }
            else {
                double market_init_time = dataMatrix[0][cdl - 1];
                if (stt == market_init_time) {
                    activated_sym[sym] = (int)cdl + 1;
                }
                else if (stt > market_init_time) {
                    hsize_t rt = (hsize_t)((stt - market_init_time) / (60 * 1000000000LL));
                    activated_sym[sym] = (int)(rt + cdl + 1);
                }
            }
        }

        stt += 60 * 1000000000LL; // 다음 분봉

    }

    // std::system("pause"); // 필요하다면 켜고, 성능에는 영향이 큼
    return 0;
}

TechnicalIndicators calculateTechnicalIndicators(
    const std::span<double>& H_p,
    const std::span<double>& L_p,
    const std::span<double>& C_p,
    const std::span<double>& V_p,
    size_t cdl)
{
    TechnicalIndicators result;

    int cndl = 10;
    int outBegIdx = 0, outNbElement = 0;

    

    // mfi
    result.mfi.resize(14);
    TA_MFI(cdl - 14, cdl - 1, H_p.data(), L_p.data(), C_p.data(), V_p.data(), 14, &outBegIdx, &outNbElement, result.mfi.data());



    // ADX
    result.adx.resize(100);
    TA_ADX(cdl - 100, cdl - 1, H_p.data(), L_p.data(), C_p.data(), 22, &outBegIdx, &outNbElement, result.adx.data());



    // TA_MAType_SMA (SMA , EMA , WMA , DEMA , TEMA , TRIMA)
    //// sar (1s)
    result.SAR_1s.resize(cndl);
    TA_SAR(cdl - cndl, cdl - 1, H_p.data(), L_p.data(), 0.1, 0.2, &outBegIdx, &outNbElement, result.SAR_1s.data()); // 주소를 가져오는거라서 주소에 상수값이 더해질경우 더 옆의 데이터도 가져와버림



    // sar (1h)
    std::vector<double> H_p_1;
    std::vector<double> L_p_1;
    for (size_t i = cdl; i >= 60; i -= 60) {
        double hourlyMax = *std::max_element(H_p.begin() + (i - 60), H_p.begin() + i);
        H_p_1.insert(H_p_1.begin(), hourlyMax);
    }
    for (size_t i = cdl; i >= 60; i -= 60) {
        double hourlyMin = *std::min_element(L_p.begin() + (i - 60), L_p.begin() + i);
        L_p_1.insert(L_p_1.begin(), hourlyMin);
    }
    result.SAR_1h.resize(H_p_1.size() - 1);
    TA_SAR(0, H_p_1.size() - 1, H_p_1.data(), L_p_1.data(), 0.1, 0.2, &outBegIdx, &outNbElement, result.SAR_1h.data());




    // stoch
    result.SlowK.resize(cndl);
    result.SlowD.resize(cndl);
    TA_STOCHF(cdl - cndl, cdl - 1, H_p.data(), L_p.data(), C_p.data(), 10, 5, TA_MAType_SMA, &outBegIdx, &outNbElement, result.SlowK.data(), result.SlowD.data());



    // bol
    result.UpperBand.resize(cdl);
    result.MiddleBand.resize(cdl);
    result.LowerBand.resize(cdl);
    TA_BBANDS(0, cdl - 1, C_p.data(), 10, 1.0, 1.0, TA_MAType_KAMA, &outBegIdx, &outNbElement, result.UpperBand.data(), result.MiddleBand.data(), result.LowerBand.data());


    // vwap
    int vwap_cdl = 50;
    double numerator = 0.0;
    double denominator = 0.0;
    for (size_t i = cdl - vwap_cdl; i < cdl; ++i) {
        numerator += C_p[i] * V_p[i];
        denominator += V_p[i];
    }
    result.VWAP = denominator != 0.0 ? numerator / denominator : 0.0;

    return result;
}
