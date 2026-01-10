this is strategy tester. (back tester)

Compared to the previous backtester written in Python, this new version is approximately 84 times faster.


data_collector.py ==============================================

The data_collector.py script is used to gather data for all coins in the futures market, collecting OHLCV and other relevant information from the moment each coin was listed up to the present. This data is then stored in a file called Backtest_Market_data.h5. The first time you run the script, it can take anywhere from 4 to 7 days to collect all the data, and once the process starts, you must not stop it—otherwise, you'll have to start over.

If you just need to update the existing data, you can simply run data_collector.py again. It will automatically detect and add only the new data without re-downloading everything.

Once Backtest_Market_data.h5 is ready, you’ll need to set its absolute path in st_tester.cpp on line 49. When you run st_tester.cpp, it will automatically detect the data file and use it to perform the strategy backtest.

================================================================

st_tester.sln ==================================================

To explain the code, on line 69, you can specify which coins to test. For instance, if you want to trade BTC, XRP, and ETH, set the value on line 313 to 3 and input those coins on line 69. Line 308 sets the total asset amount for testing, line 309 defines the initial purchase amount, and line 310 specifies the additional purchase amount (which could be removed in the development phase). Line 315 defines the maximum number of additional purchases, while lines 316 to 330 are hyperparameters for the strategy, which aren't crucial. Line 20 defines the testing start time; for example, setting int a = 2019, int b = 10, int c = 1, int d = 20, int e = 21 will start testing from 8:21 PM on October 1, 2019. The if condition on line 402 and the else condition on line 589 determine whether a coin has been purchased. If no purchase has been made, the strategy in the else statement will determine the initial buy, and the if statement will handle additional purchases, profit-taking, or stop-loss decisions. The calculateTechnicalIndicators function on line 660 uses ta_libc to calculate and return technical indicators. You can add more indicators by using talib and returning them in this function. Functions on lines 217 and 252 handle long buy and sell orders, with the initial total asset value stored in wallet["balance"][0] (set on line 308). The wallet unordered map stores the information of the purchased coins. Similarly, lines 142 and 181 manage short buy and sell orders, with the logic resembling that of the long buy and sell functions on lines 217 and 252.

Additionally, the time_printer(stt, UTC_C); code can be very useful in the main function. It prints the current time of the backtester. If you're developing the algorithm and want to determine the exact moment an event occurs, you can simply add time_printer(stt, UTC_C); at the appropriate line in your code. This will help you track when specific events happen during the backtesting process.

=================================================================

algo ============================================================

In the st_tester.cpp code, the algorithm I’ve developed detects volume and overbought/oversold conditions to identify if the current price is at a low point during a downtrend, which allows it to catch lows effectively. However, the problem lies in its inability to properly handle sell orders and stop-losses, resulting in significant losses. 

At line 595, the algorithm first uses Indicator A(MFI < 13) to decide if it's appropriate to enter a buy position. If it passes, the script saves the data in wallet[sym] to mark that the first buy entry was successful. The data stored is { -1 , time_delay_nb , 0 , 0 , 0 , 0 , 0 }. Here:

[0] indicates the current status of the coin.
time_delay_nb is set to 10, meaning that if a second buy doesn’t happen within 10 minutes, it will be canceled.
The other zeros are placeholders for additional buys (split buys).
If the first buy passes, the process continues through line 402 and goes to line 409, where Indicator B(ADX > 40) is used to check if it’s still appropriate to enter a buy. If the conditions are met, it changes wallet[sym][0] to -2, allowing for a split buy. At line 444, the split buy takes place.

If the split buy isn’t completely filled within 1 minute, all the orders are canceled. However, if only part of the order is filled, only the filled portion gets processed, and the corresponding amount is added to wallet[sym] (this happens at line 452 when the BUY function is called).

From line 491 onward, the algorithm handles additional buys, taking profits, and stop-losses. But honestly, the additional buy, profit-taking, and stop-loss algorithms are not working well, and I’m not satisfied with them. These parts definitely need more research and improvement.

===================================================================

By the way, if you're curious about how the chart looks at the time of purchase, I don't have a chart display feature implemented right now. However, you can visit this link, and by copying and pasting the purchase date, you’ll be able to find the exact chart and visualize it.

If there's anything I've missed or anything you think I might have forgotten while explaining, feel free to ask!
